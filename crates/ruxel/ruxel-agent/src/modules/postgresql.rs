#![expect(
    clippy::cast_possible_truncation,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::let_underscore_must_use,
    clippy::unwrap_used,
    reason = "replaced by the Ruxel rewrite"
)]

use super::{ExecContext, become_command, params_object, str_param};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};

fn login_port(obj: &serde_json::Map<String, Value>) -> String {
    obj.get("login_port").map_or_else(
        || "5432".into(),
        |v| match v {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => "5432".into(),
        },
    )
}

fn psql(ctx: &ExecContext, port: &str, db: Option<&str>, sql: &str) -> Result<String, String> {
    let key = SessionKey {
        become_user: ctx.become_user.clone(),
        port: port.to_string(),
        database: db.map(str::to_string),
    };
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if !sessions.contains_key(&key) {
            sessions.insert(key.clone(), PsqlSession::spawn(ctx, &key)?);
        }
        let result = sessions.get_mut(&key).expect("session inserted").query(sql);
        if result.is_err() {
            sessions.remove(&key);
        }
        result
    })
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct SessionKey {
    become_user: Option<String>,
    port: String,
    database: Option<String>,
}

struct PsqlSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
    stderr_thread: Option<std::thread::JoinHandle<()>>,
    sequence: u64,
}

thread_local! {
    static SESSIONS: RefCell<HashMap<SessionKey, PsqlSession>> = RefCell::new(HashMap::new());
}

pub(super) fn shutdown() {
    SESSIONS.with(|sessions| sessions.borrow_mut().clear());
}

impl PsqlSession {
    fn spawn(ctx: &ExecContext, key: &SessionKey) -> Result<Self, String> {
        let mut args = vec!["-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-p", &key.port];
        if let Some(database) = &key.database {
            args.push("-d");
            args.push(database);
        }
        let mut child = become_command(ctx, "psql", &args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("exec psql: {error}"))?;
        let stdin = child.stdin.take().ok_or("psql stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("psql stdout")?);
        let mut child_stderr = child.stderr.take().ok_or("psql stderr")?;
        let stderr = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&stderr);
        let stderr_thread = std::thread::spawn(move || {
            use std::io::Read as _;
            let mut text = String::new();
            let _ = child_stderr.read_to_string(&mut text);
            *captured.lock().unwrap() = text;
        });
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            stderr_thread: Some(stderr_thread),
            sequence: 0,
        })
    }

    fn query(&mut self, sql: &str) -> Result<String, String> {
        self.sequence += 1;
        let marker = format!(
            "__RUXEL_QUERY_END_{}_{}__",
            std::process::id(),
            self.sequence
        );
        self.stdin
            .write_all(sql.as_bytes())
            .and_then(|()| {
                if sql.trim_end().ends_with(';') {
                    self.stdin.write_all(b"\n")
                } else {
                    self.stdin.write_all(b";\n")
                }
            })
            .and_then(|()| {
                self.stdin
                    .write_all(format!("\\echo {marker}\n").as_bytes())
            })
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("psql stdin: {error}"))?;

        let mut output = String::new();
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("psql stdout: {error}"))?;
            if read == 0 {
                if let Some(thread) = self.stderr_thread.take() {
                    let _ = thread.join();
                }
                let error = self.stderr.lock().unwrap().trim().to_string();
                return Err(if error.is_empty() {
                    "psql exited before query completed".into()
                } else {
                    format!("psql failed: {error}")
                });
            }
            if line.trim_end() == marker {
                return Ok(output.trim().to_string());
            }
            output.push_str(&line);
        }
    }
}

impl Drop for PsqlSession {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"\\q\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn validate_privs(privs: &str) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
        "USAGE",
        "CREATE",
        "CONNECT",
        "TEMPORARY",
        "TEMP",
        "EXECUTE",
        "MAINTAIN",
        "ALL",
        "ALL PRIVILEGES",
    ];
    for p in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !ALLOWED.contains(&p.to_uppercase().as_str()) {
            return Err(format!(
                "postgresql_privs: privilege {p:?} outside the closed surface"
            ));
        }
    }
    Ok(())
}

fn acl_privileges(privs: &str, object_type: &str, server_version: u32) -> Vec<String> {
    let all: &[&str] = match object_type {
        "database" => &["CREATE", "CONNECT", "TEMPORARY"],
        "schema" => &["CREATE", "USAGE"],
        "table" | "default_privs" => &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
        ],
        _ => &[],
    };
    let mut result = Vec::new();
    for privilege in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let privilege = privilege.to_uppercase();
        if privilege == "ALL" || privilege == "ALL PRIVILEGES" {
            result.extend(all.iter().map(|p| (*p).to_owned()));
            if matches!(object_type, "table" | "default_privs") && server_version >= 170_000 {
                result.push("MAINTAIN".to_owned());
            }
        } else if privilege == "TEMP" {
            result.push("TEMPORARY".to_owned());
        } else {
            result.push(privilege);
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

fn validate_role_attr_flags(flags: &str) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "SUPERUSER",
        "NOSUPERUSER",
        "CREATEDB",
        "NOCREATEDB",
        "CREATEROLE",
        "NOCREATEROLE",
        "INHERIT",
        "NOINHERIT",
        "LOGIN",
        "NOLOGIN",
        "REPLICATION",
        "NOREPLICATION",
        "BYPASSRLS",
        "NOBYPASSRLS",
    ];
    for flag in role_flag_tokens(flags) {
        if !ALLOWED.contains(&flag.as_str()) {
            return Err(format!(
                "postgresql_user: role attribute {flag:?} outside the closed surface"
            ));
        }
    }
    Ok(())
}

fn role_flag_tokens(flags: &str) -> Vec<String> {
    flags
        .split([',', ' '])
        .filter(|token| !token.is_empty())
        .map(str::to_uppercase)
        .collect()
}

fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub(super) fn db(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("postgresql_db: name required")?;
    let owner = str_param(obj, "owner");
    let state = str_param(obj, "state").unwrap_or("present");
    let port = login_port(obj);

    let exists = psql(
        ctx,
        &port,
        None,
        &format!("SELECT 1 FROM pg_database WHERE datname={}", lit(name)),
    )? == "1";

    let mut changed = false;
    if state != "present" {
        return Err(format!(
            "postgresql_db: state {state:?} outside the closed surface"
        ));
    }

    if !exists {
        changed = true;
        if !ctx.check_mode {
            let mut sql = format!("CREATE DATABASE {}", ident(name));
            if let Some(o) = owner {
                sql.push_str(&format!(" OWNER {}", ident(o)));
            }
            psql(ctx, &port, None, &sql)?;
        }
    } else if let Some(o) = owner {
        let current = psql(
            ctx,
            &port,
            None,
            &format!(
                "SELECT pg_catalog.pg_get_userbyid(datdba) FROM pg_database WHERE datname={}",
                lit(name)
            ),
        )?;
        if current != o {
            changed = true;
            if !ctx.check_mode {
                psql(
                    ctx,
                    &port,
                    None,
                    &format!("ALTER DATABASE {} OWNER TO {}", ident(name), ident(o)),
                )?;
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "db": name}))
}

pub(super) fn user(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("postgresql_user: name required")?;
    let password = str_param(obj, "password");
    let role_attr_flags = str_param(obj, "role_attr_flags");
    let state = str_param(obj, "state").unwrap_or("present");
    let port = login_port(obj);
    if state != "present" {
        return Err(format!(
            "postgresql_user: state {state:?} outside the closed surface"
        ));
    }
    if let Some(flags) = role_attr_flags {
        validate_role_attr_flags(flags)?;
    }

    let exists = psql(
        ctx,
        &port,
        None,
        &format!("SELECT 1 FROM pg_roles WHERE rolname={}", lit(name)),
    )? == "1";

    let mut changed = false;

    if exists {
        if let Some(f) = role_attr_flags
            && flags_changed(ctx, &port, name, f)?
        {
            changed = true;
            if !ctx.check_mode {
                psql(
                    ctx,
                    &port,
                    None,
                    &format!("ALTER ROLE {} {}", ident(name), flags_to_sql(f)),
                )?;
            }
        }
        if let Some(p) = password
            && password_changed(ctx, &port, name, p)?
        {
            changed = true;
            if !ctx.check_mode {
                psql(
                    ctx,
                    &port,
                    None,
                    &format!("ALTER ROLE {} PASSWORD {}", ident(name), lit(p)),
                )?;
            }
        }
    } else {
        changed = true;
        if !ctx.check_mode {
            let mut sql = format!("CREATE ROLE {}", ident(name));
            let flags = role_attr_flags.map_or_else(|| "LOGIN".into(), flags_to_sql);
            if !flags.is_empty() {
                sql.push(' ');
                sql.push_str(&flags);
            }
            if let Some(p) = password {
                sql.push_str(&format!(" PASSWORD {}", lit(p)));
            }
            psql(ctx, &port, None, &sql)?;
        }
    }

    Ok(json!({"changed": changed, "failed": false, "user": name}))
}

fn password_changed(
    ctx: &ExecContext,
    port: &str,
    name: &str,
    password: &str,
) -> Result<bool, String> {
    let stored = psql(
        ctx,
        port,
        None,
        &format!(
            "SELECT rolpassword FROM pg_authid WHERE rolname={}",
            lit(name)
        ),
    )?;
    if stored.is_empty() {
        return Ok(true);
    }
    if !stored.starts_with("SCRAM-SHA-256$") {
        return Ok(true);
    }
    let body = &stored["SCRAM-SHA-256$".len()..];
    let (iter_salt, keys) = body.split_once('$').ok_or("malformed scram verifier")?;
    let (iter, salt) = iter_salt
        .split_once(':')
        .ok_or("malformed scram iter:salt")?;
    let stored_stored_key = keys.split(':').next().unwrap_or("");
    let computed = scram_stored_key(password, salt, iter.parse().unwrap_or(4096))?;
    Ok(computed != stored_stored_key)
}

fn scram_stored_key(password: &str, b64salt: &str, iterations: u32) -> Result<String, String> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::{Digest, Sha256};
    type H = Hmac<Sha256>;

    let salt = b64_decode(b64salt).ok_or("bad scram salt b64")?;
    let mut salted = [0u8; 32];
    pbkdf2::pbkdf2::<H>(password.as_bytes(), &salt, iterations, &mut salted)
        .map_err(|_| "pbkdf2".to_string())?;
    let mut mac = H::new_from_slice(&salted).map_err(|_| "hmac key")?;
    mac.update(b"Client Key");
    let client_key = mac.finalize().into_bytes();
    let stored_key = Sha256::digest(client_key);
    Ok(b64_encode(&stored_key))
}

fn flags_to_sql(flags: &str) -> String {
    flags
        .split([',', ' '])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn flags_changed(ctx: &ExecContext, port: &str, name: &str, flags: &str) -> Result<bool, String> {
    let row = psql(
        ctx,
        port,
        None,
        &format!(
            "SELECT rolsuper,rolcreaterole,rolcreatedb,rolinherit,rolcanlogin,rolreplication,rolbypassrls FROM pg_roles WHERE rolname={}",
            lit(name)
        ),
    )?;
    let cols: Vec<&str> = row.split('|').collect();
    let mappings = [
        ("SUPERUSER", 0),
        ("CREATEROLE", 1),
        ("CREATEDB", 2),
        ("INHERIT", 3),
        ("LOGIN", 4),
        ("REPLICATION", 5),
        ("BYPASSRLS", 6),
    ];
    for (flag, column) in mappings {
        if let Some(wanted) = wanted_flag(flags, flag) {
            let actual = match cols.get(column) {
                Some(&"t") => true,
                Some(&"f") => false,
                _ => return Err("postgresql_user: malformed pg_roles flag row".into()),
            };
            if actual != wanted {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn wanted_flag(flags: &str, positive: &str) -> Option<bool> {
    let tokens = role_flag_tokens(flags);
    let negative = format!("NO{positive}");
    if tokens.iter().any(|flag| flag == &negative) {
        Some(false)
    } else if tokens.iter().any(|flag| flag == positive) {
        Some(true)
    } else {
        None
    }
}

pub(super) fn schema(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("postgresql_schema: name required")?;
    let login_db = str_param(obj, "login_db").ok_or("postgresql_schema: login_db required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    let owner = str_param(obj, "owner");
    let port = login_port(obj);
    if state != "present" {
        return Err(format!(
            "postgresql_schema: state {state:?} outside the closed surface"
        ));
    }

    let current_owner = psql(
        ctx,
        &port,
        Some(login_db),
        &format!(
            "SELECT r.rolname FROM pg_namespace n JOIN pg_roles r ON r.oid=n.nspowner WHERE n.nspname={}",
            lit(name)
        ),
    )?;

    let mut changed = false;
    if current_owner.is_empty() {
        changed = true;
        if !ctx.check_mode {
            let authorization = owner
                .map(|value| format!(" AUTHORIZATION {}", ident(value)))
                .unwrap_or_default();
            psql(
                ctx,
                &port,
                Some(login_db),
                &format!("CREATE SCHEMA {}{authorization}", ident(name)),
            )?;
        }
    } else if let Some(owner) = owner
        && current_owner != owner
    {
        changed = true;
        if !ctx.check_mode {
            psql(
                ctx,
                &port,
                Some(login_db),
                &format!("ALTER SCHEMA {} OWNER TO {}", ident(name), ident(owner)),
            )?;
        }
    }
    Ok(json!({"changed": changed, "failed": false, "schema": name}))
}

pub(super) fn privs(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let login_db = str_param(obj, "login_db").ok_or("postgresql_privs: login_db required")?;
    let role = str_param(obj, "role").ok_or("postgresql_privs: role required")?;
    let typ = str_param(obj, "type").unwrap_or("table");
    let privs_list = str_param(obj, "privs").unwrap_or("");
    let state = str_param(obj, "state").unwrap_or("present");
    let objs = str_param(obj, "objs");
    let schema = str_param(obj, "schema");
    let target_roles = str_param(obj, "target_roles");
    let port = login_port(obj);
    if state != "present" {
        return Err(format!(
            "postgresql_privs: state {state:?} outside the closed surface"
        ));
    }
    validate_privs(privs_list)?;
    if target_roles.is_some() && typ != "default_privs" {
        return Err("postgresql_privs: target_roles requires type=default_privs".into());
    }

    let needs_version = matches!(typ, "table" | "default_privs")
        && privs_list
            .split(',')
            .map(str::trim)
            .any(|p| p.eq_ignore_ascii_case("ALL") || p.eq_ignore_ascii_case("ALL PRIVILEGES"));
    let server_version = if needs_version {
        psql(ctx, &port, Some(login_db), "SHOW server_version_num")?
            .parse::<u32>()
            .map_err(|_| "postgresql_privs: invalid server_version_num")?
    } else {
        0
    };
    let probes = acl_privileges(privs_list, typ, server_version);

    let needed = match typ {
        "database" => privs_missing_database(
            ctx,
            &port,
            objs.ok_or("postgresql_privs: objs required for type=database")?,
            role,
            &probes,
        )?,
        "schema" => privs_missing_schema(ctx, &port, login_db, role, objs.unwrap_or(""), &probes)?,
        "table" => privs_missing_table(
            ctx,
            &port,
            login_db,
            role,
            objs.unwrap_or(""),
            schema,
            &probes,
        )?,
        "default_privs" => privs_missing_default(
            ctx,
            &port,
            login_db,
            role,
            schema.unwrap_or("public"),
            &probes,
            target_roles,
        )?,
        other => {
            return Err(format!(
                "postgresql_privs: type {other:?} outside the closed surface"
            ));
        }
    };

    let mut changed = false;
    if needed {
        changed = true;
        if !ctx.check_mode {
            for sql in grant_sql(typ, role, privs_list, objs, schema, target_roles)? {
                psql(ctx, &port, Some(login_db), &sql)?;
            }
        }
    }
    Ok(json!({"changed": changed, "failed": false, "role": role, "type": typ}))
}

fn privs_missing_database(
    ctx: &ExecContext,
    port: &str,
    objs: &str,
    role: &str,
    privs: &[String],
) -> Result<bool, String> {
    for db in objs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        for p in privs {
            let held = psql(
                ctx,
                port,
                None,
                &format!(
                    "SELECT 1 FROM pg_database d, aclexplode(d.datacl) a \
                     WHERE d.datname={} AND a.grantee=(SELECT oid FROM pg_roles WHERE rolname={}) \
                     AND a.privilege_type={} LIMIT 1",
                    lit(db),
                    lit(role),
                    lit(&p.to_uppercase())
                ),
            )?;
            if held != "1" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn privs_missing_schema(
    ctx: &ExecContext,
    port: &str,
    db: &str,
    role: &str,
    objs: &str,
    privs: &[String],
) -> Result<bool, String> {
    for schema in objs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        for p in privs {
            let held = psql(
                ctx,
                port,
                Some(db),
                &format!(
                    "SELECT 1 FROM pg_namespace n, aclexplode(n.nspacl) a \
                     WHERE n.nspname={} AND a.grantee=(SELECT oid FROM pg_roles WHERE rolname={}) \
                     AND a.privilege_type={} LIMIT 1",
                    lit(schema),
                    lit(role),
                    lit(&p.to_uppercase())
                ),
            )?;
            if held != "1" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn privs_missing_table(
    ctx: &ExecContext,
    port: &str,
    db: &str,
    role: &str,
    objs: &str,
    schema: Option<&str>,
    privs: &[String],
) -> Result<bool, String> {
    let role_oid_sql = format!("(SELECT oid FROM pg_roles WHERE rolname={})", lit(role));
    let tables: Vec<(String, String)> = if objs == "ALL_IN_SCHEMA" {
        let s = schema.ok_or("postgresql_privs: schema required for ALL_IN_SCHEMA")?;
        let rows = psql(
            ctx,
            port,
            Some(db),
            &format!(
                "SELECT schemaname,tablename FROM pg_tables WHERE schemaname={}",
                lit(s)
            ),
        )?;
        rows.lines()
            .filter_map(|l| {
                l.split_once('|')
                    .map(|(a, b)| (a.to_string(), b.to_string()))
            })
            .collect()
    } else {
        let s = schema.unwrap_or("public");
        objs.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| (s.to_string(), t.to_string()))
            .collect()
    };
    if tables.is_empty() {
        return Ok(false);
    }
    for (sch, tbl) in &tables {
        for p in privs {
            let held = psql(
                ctx,
                port,
                Some(db),
                &format!(
                    "SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace, \
                     aclexplode(c.relacl) a \
                     WHERE n.nspname={} AND c.relname={} AND a.grantee={} \
                     AND a.privilege_type={} LIMIT 1",
                    lit(sch),
                    lit(tbl),
                    role_oid_sql,
                    lit(&p.to_uppercase())
                ),
            )?;
            if held != "1" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn default_priv_query(
    role: &str,
    schema: &str,
    privilege: &str,
    target_role: Option<&str>,
) -> String {
    let owner = target_role.map_or_else(
        || "(SELECT oid FROM pg_roles WHERE rolname=current_user)".into(),
        |role| format!("(SELECT oid FROM pg_roles WHERE rolname={})", lit(role)),
    );
    format!(
        "SELECT 1 FROM pg_default_acl d \
         JOIN pg_namespace n ON n.oid=d.defaclnamespace, \
         aclexplode(d.defaclacl) a \
         WHERE d.defaclobjtype='r' AND n.nspname={} \
         AND d.defaclrole={owner} \
         AND a.grantee=(SELECT oid FROM pg_roles WHERE rolname={}) \
         AND a.privilege_type={} LIMIT 1",
        lit(schema),
        lit(role),
        lit(&privilege.to_uppercase())
    )
}

fn privs_missing_default(
    ctx: &ExecContext,
    port: &str,
    db: &str,
    role: &str,
    schema: &str,
    privs: &[String],
    target_roles: Option<&str>,
) -> Result<bool, String> {
    let targets: Vec<Option<&str>> = match target_roles {
        Some(roles) => roles
            .split(',')
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .map(Some)
            .collect(),
        None => vec![None],
    };
    for target in targets {
        for privilege in privs {
            let held = psql(
                ctx,
                port,
                Some(db),
                &default_priv_query(role, schema, privilege, target),
            )?;
            if held != "1" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn grant_sql(
    typ: &str,
    role: &str,
    privs: &str,
    objs: Option<&str>,
    schema: Option<&str>,
    target_roles: Option<&str>,
) -> Result<Vec<String>, String> {
    let r = ident(role);
    Ok(match typ {
        "database" => objs
            .ok_or("postgresql_privs: objs required for type=database")?
            .split(',')
            .map(str::trim)
            .filter(|db| !db.is_empty())
            .map(|db| format!("GRANT {privs} ON DATABASE {} TO {r}", ident(db)))
            .collect(),
        "schema" => objs
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("GRANT {privs} ON SCHEMA {} TO {r}", ident(s)))
            .collect(),
        "table" => {
            let o = objs.unwrap_or("");
            if o == "ALL_IN_SCHEMA" {
                let s = schema.ok_or("postgresql_privs: schema required")?;
                vec![format!(
                    "GRANT {privs} ON ALL TABLES IN SCHEMA {} TO {r}",
                    ident(s)
                )]
            } else {
                let s = schema.unwrap_or("public");
                o.split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(|t| format!("GRANT {privs} ON TABLE {}.{} TO {r}", ident(s), ident(t)))
                    .collect()
            }
        }
        "default_privs" => {
            let s = schema.unwrap_or("public");
            match target_roles {
                Some(roles) => roles
                    .split(',')
                    .map(str::trim)
                    .filter(|role| !role.is_empty())
                    .map(|target| {
                        format!(
                            "ALTER DEFAULT PRIVILEGES FOR ROLE {} IN SCHEMA {} GRANT {privs} ON TABLES TO {r}",
                            ident(target),
                            ident(s)
                        )
                    })
                    .collect(),
                None => vec![format!(
                    "ALTER DEFAULT PRIVILEGES IN SCHEMA {} GRANT {privs} ON TABLES TO {r}",
                    ident(s)
                )],
            }
        }
        other => {
            return Err(format!(
                "postgresql_privs: type {other:?} outside the closed surface"
            ));
        }
    })
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let s: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::new();
    for chunk in s.chunks(4) {
        let mut acc = 0u32;
        let mut bits = 0u32;
        for &c in chunk {
            acc = (acc << 6) | u32::from(val(c)?);
            bits += 6;
        }
        let nbytes = bits / 8;
        acc >>= bits % 8;
        for i in (0..nbytes).rev() {
            out.push((acc >> (i * 8)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_probes_expand_all_and_normalize_temp() {
        for (kind, held) in [
            ("database", vec!["CONNECT", "CREATE", "TEMPORARY"]),
            ("schema", vec!["CREATE", "USAGE"]),
            (
                "table",
                vec![
                    "DELETE",
                    "INSERT",
                    "REFERENCES",
                    "SELECT",
                    "TRIGGER",
                    "TRUNCATE",
                    "UPDATE",
                ],
            ),
            (
                "default_privs",
                vec![
                    "DELETE",
                    "INSERT",
                    "REFERENCES",
                    "SELECT",
                    "TRIGGER",
                    "TRUNCATE",
                    "UPDATE",
                ],
            ),
        ] {
            for requested in ["ALL", "all privileges"] {
                let probes = super::acl_privileges(requested, kind, 160_000);
                assert_eq!(probes, held);
                assert!(
                    probes
                        .iter()
                        .all(|privilege| held.contains(&privilege.as_str()))
                );
                assert!(
                    probes
                        .iter()
                        .any(|privilege| !held[1..].contains(&privilege.as_str()))
                );
            }
        }
        assert_eq!(
            super::acl_privileges("temp, TEMPORARY", "database", 180_000),
            ["TEMPORARY"]
        );
    }

    #[test]
    fn all_table_acl_probes_follow_server_version() {
        for kind in ["table", "default_privs"] {
            let old = super::acl_privileges("ALL", kind, 160_000);
            let current = super::acl_privileges("ALL PRIVILEGES", kind, 170_000);
            assert!(!old.iter().any(|p| p == "MAINTAIN"));
            assert!(current.iter().any(|p| p == "MAINTAIN"));
            assert_eq!(current.len(), old.len() + 1);
            assert!(current.iter().any(|p| !old.contains(p)));
            assert!(super::acl_privileges("all", kind, 180_000).contains(&"MAINTAIN".into()));
        }
        assert!(super::validate_privs("MAINTAIN").is_ok());
    }

    #[test]
    fn b64_roundtrip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let enc = super::b64_encode(s.as_bytes());
            assert_eq!(super::b64_decode(&enc).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn sql_inputs_are_allowlisted_and_quoted() {
        assert!(validate_privs("SELECT,UPDATE").is_ok());
        assert!(validate_privs("SELECT; DROP TABLE users").is_err());
        assert_eq!(lit("it's"), "'it''s'");
        assert_eq!(ident("odd\"name"), "\"odd\"\"name\"");
    }

    #[test]
    fn grants_and_default_acl_queries_target_role_and_schema() {
        let sql = grant_sql(
            "table",
            "reader",
            "SELECT",
            Some("events"),
            Some("audit"),
            None,
        )
        .unwrap();
        assert_eq!(
            sql,
            vec!["GRANT SELECT ON TABLE \"audit\".\"events\" TO \"reader\""]
        );
        let query = default_priv_query("reader", "audit", "select", None);
        assert!(query.contains("n.nspname='audit'"));
        assert!(query.contains("rolname='reader'"));
        assert!(query.contains("privilege_type='SELECT'"));
    }

    #[test]
    fn default_privileges_target_object_creator_roles() {
        let query = default_priv_query("reader", "audit", "select", Some("writer"));
        assert!(query.contains("d.defaclrole=(SELECT oid FROM pg_roles WHERE rolname='writer')"));
        let sql = grant_sql(
            "default_privs",
            "reader",
            "SELECT",
            Some("TABLES"),
            Some("audit"),
            Some("writer,loader"),
        )
        .unwrap();
        assert_eq!(sql.len(), 2);
        assert!(sql[0].contains("FOR ROLE \"writer\""));
        assert!(sql[1].contains("FOR ROLE \"loader\""));
    }

    #[test]
    fn role_flag_wants_positive_and_negative_forms() {
        assert_eq!(wanted_flag("LOGIN,CREATEDB", "LOGIN"), Some(true));
        assert_eq!(wanted_flag("NOLOGIN", "LOGIN"), Some(false));
        assert_eq!(wanted_flag("CREATEDB", "LOGIN"), None);
    }

    #[test]
    fn scram_stored_key_matches_postgres() {
        let got =
            super::scram_stored_key("s3cr3t-ruxel", "tmEKTUhvHycriHKDSR74nA==", 4096).unwrap();
        assert_eq!(got, "qsXMYQ6PkvDbKdO/Fwo5aAQisdU9bG3fdoLEpQMpraM=");
    }

    #[test]
    fn b64_decode_partial_groups() {
        assert_eq!(super::b64_decode("nA==").unwrap(), vec![0x9c]);
        assert_eq!(super::b64_decode("tA==").unwrap().len(), 1);
    }

    #[test]
    fn role_attribute_allowlist_accepts_boolean_flags() {
        assert!(validate_role_attr_flags("SUPERUSER,CREATEDB").is_ok());
        assert!(validate_role_attr_flags("NOLOGIN").is_ok());
    }

    #[test]
    fn role_attribute_allowlist_rejects_statement_injection() {
        assert!(validate_role_attr_flags("SUPERUSER; DROP ROLE synthetic").is_err());
        assert!(validate_role_attr_flags("CONNECTION LIMIT 5").is_err());
    }

    #[test]
    fn wanted_role_flag_handles_positive_negative_and_absent() {
        assert_eq!(wanted_flag("CREATEDB", "CREATEDB"), Some(true));
        assert_eq!(wanted_flag("NOCREATEDB", "CREATEDB"), Some(false));
        assert_eq!(wanted_flag("LOGIN", "CREATEDB"), None);
    }

    #[test]
    fn default_privilege_query_targets_explicit_table_acl() {
        let query = default_priv_query("synthetic_role", "public", "select", None);
        assert!(query.contains("pg_default_acl"));
        assert!(query.contains("aclexplode(d.defaclacl)"));
        assert!(query.contains("d.defaclobjtype='r'"));
        assert!(query.contains("a.grantee=(SELECT oid FROM pg_roles"));
        assert!(query.contains("a.privilege_type='SELECT'"));
    }
}

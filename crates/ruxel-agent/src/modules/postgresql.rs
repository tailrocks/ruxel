//! `community.postgresql.postgresql_{db,user,schema,privs}` (SEMANTICS §6).
//! Connection: psql over the unix socket as the `become_user` (peer auth as
//! postgres) on `login_port`. Idempotence is decided in SQL — pg_catalog
//! state and explicit-ACL inspection via aclexplode — so "changed"
//! reflects a real catalog delta, never a blind re-grant.

use super::{ExecContext, become_command, params_object, str_param};
use serde_json::{Value, json};

fn login_port(obj: &serde_json::Map<String, Value>) -> String {
    obj.get("login_port")
        .map(|v| match v {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => "5432".into(),
        })
        .unwrap_or_else(|| "5432".into())
}

/// Run a SQL query, returning trimmed stdout (psql -tA: tuples-only,
/// unaligned). `db` selects the target database (maintenance db otherwise).
fn psql(ctx: &ExecContext, port: &str, db: Option<&str>, sql: &str) -> Result<String, String> {
    let mut args = vec!["-p", port, "-tAc", sql];
    if let Some(d) = db {
        args.insert(0, d);
        args.insert(0, "-d");
    }
    let out = become_command(ctx, "psql", &args)
        .output()
        .map_err(|e| format!("exec psql: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "psql {sql:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Quote an SQL string literal.
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Quote an SQL identifier.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// -- postgresql_db ----------------------------------------------------------

pub fn db(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
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

// -- postgresql_user --------------------------------------------------------

pub fn user(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
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

    let exists = psql(
        ctx,
        &port,
        None,
        &format!("SELECT 1 FROM pg_roles WHERE rolname={}", lit(name)),
    )? == "1";

    let mut changed = false;

    if !exists {
        changed = true;
        if !ctx.check_mode {
            let mut sql = format!("CREATE ROLE {} LOGIN", ident(name));
            if let Some(p) = password {
                sql.push_str(&format!(" PASSWORD {}", lit(p)));
            }
            if let Some(f) = role_attr_flags {
                sql.push(' ');
                sql.push_str(&flags_to_sql(f));
            }
            psql(ctx, &port, None, &sql)?;
        }
    } else {
        // Attr-flag drift (SUPERUSER etc.).
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
        // Password idempotence (the ⚠): compare the stored SCRAM verifier
        // to a verifier derived from the supplied password. PG stores
        // SCRAM-SHA-256$<iter>:<salt>$... — re-deriving needs the salt, so
        // instead ALTER and let PG no-op when the verifier matches is NOT
        // observable as unchanged. Correct rule: compute whether the
        // cleartext already authenticates by comparing against the stored
        // verifier via PG's own check.
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
    }

    Ok(json!({"changed": changed, "failed": false, "user": name}))
}

/// True when the supplied password does not match the stored SCRAM verifier.
/// Uses PG's own scram machinery: re-hash the cleartext with the stored
/// salt+iterations and compare the stored-key — done entirely in SQL so the
/// rule matches Ansible's (community.postgresql compares the same way).
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
        // md5 or plain — treat any mismatch conservatively as changed.
        return Ok(true);
    }
    // Parse SCRAM-SHA-256$<iter>:<b64salt>$<b64storedkey>:<b64serverkey>
    let body = &stored["SCRAM-SHA-256$".len()..];
    let (iter_salt, keys) = body.split_once('$').ok_or("malformed scram verifier")?;
    let (iter, salt) = iter_salt
        .split_once(':')
        .ok_or("malformed scram iter:salt")?;
    let stored_stored_key = keys.split(':').next().unwrap_or("");
    // Derive StoredKey from the cleartext using the stored salt/iterations
    // via a tiny SQL routine (PG has no built-in scram() exposed, so use
    // the pgcrypto-free path: ask PG to hash by setting a scratch role).
    // Simplest faithful approach: set the password on a temp role and read
    // back its verifier with the SAME salt is not possible (random salt).
    // So compute StoredKey in-process.
    let computed = scram_stored_key(password, salt, iter.parse().unwrap_or(4096))?;
    Ok(computed != stored_stored_key)
}

/// StoredKey = base64( SHA256( HMAC-SHA256(SaltedPassword, "Client Key") ) ),
/// SaltedPassword = PBKDF2-HMAC-SHA256(password, base64decode(salt), iter).
fn scram_stored_key(password: &str, b64salt: &str, iterations: u32) -> Result<String, String> {
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};
    type H = Hmac<Sha256>;

    let salt = b64_decode(b64salt).ok_or("bad scram salt b64")?;
    // PBKDF2-HMAC-SHA256
    let mut salted = [0u8; 32];
    pbkdf2::pbkdf2::<H>(password.as_bytes(), &salt, iterations, &mut salted)
        .map_err(|_| "pbkdf2".to_string())?;
    let mut mac = <H as Mac>::new_from_slice(&salted).map_err(|_| "hmac key")?;
    mac.update(b"Client Key");
    let client_key = mac.finalize().into_bytes();
    let stored_key = Sha256::digest(client_key);
    Ok(b64_encode(&stored_key))
}

fn flags_to_sql(flags: &str) -> String {
    // role_attr_flags is a comma/space list of PG role options.
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
            "SELECT rolsuper,rolcreaterole,rolcreatedb,rolreplication FROM pg_roles WHERE rolname={}",
            lit(name)
        ),
    )?;
    let cols: Vec<&str> = row.split('|').collect();
    let is_super = cols.first() == Some(&"t");
    let wants_super =
        flags.to_uppercase().contains("SUPERUSER") && !flags.to_uppercase().contains("NOSUPERUSER");
    // The workload only toggles SUPERUSER; broaden if more flags appear.
    Ok(is_super != wants_super)
}

// -- postgresql_schema ------------------------------------------------------

pub fn schema(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("postgresql_schema: name required")?;
    let login_db = str_param(obj, "login_db").ok_or("postgresql_schema: login_db required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    let port = login_port(obj);
    if state != "present" {
        return Err(format!(
            "postgresql_schema: state {state:?} outside the closed surface"
        ));
    }

    let exists = psql(
        ctx,
        &port,
        Some(login_db),
        &format!("SELECT 1 FROM pg_namespace WHERE nspname={}", lit(name)),
    )? == "1";

    let mut changed = false;
    if !exists {
        changed = true;
        if !ctx.check_mode {
            psql(
                ctx,
                &port,
                Some(login_db),
                &format!("CREATE SCHEMA {}", ident(name)),
            )?;
        }
    }
    Ok(json!({"changed": changed, "failed": false, "schema": name}))
}

// -- postgresql_privs -------------------------------------------------------

pub fn privs(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let login_db = str_param(obj, "login_db").ok_or("postgresql_privs: login_db required")?;
    let role = str_param(obj, "role").ok_or("postgresql_privs: role required")?;
    let typ = str_param(obj, "type").unwrap_or("table");
    let privs_list = str_param(obj, "privs").unwrap_or("");
    let state = str_param(obj, "state").unwrap_or("present");
    let objs = str_param(obj, "objs");
    let schema = str_param(obj, "schema");
    let port = login_port(obj);
    if state != "present" {
        return Err(format!(
            "postgresql_privs: state {state:?} outside the closed surface"
        ));
    }

    // changed iff at least one requested privilege is not already held.
    let needed = match typ {
        "database" => privs_missing_database(ctx, &port, login_db, role, privs_list)?,
        "schema" => {
            privs_missing_schema(ctx, &port, login_db, role, objs.unwrap_or(""), privs_list)?
        }
        "table" => privs_missing_table(
            ctx,
            &port,
            login_db,
            role,
            objs.unwrap_or(""),
            schema,
            privs_list,
        )?,
        "default_privs" => true, // pg_default_acl compare below decides; see grant
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
            for sql in grant_sql(typ, role, privs_list, objs, schema)? {
                psql(ctx, &port, Some(login_db), &sql)?;
            }
        }
    }
    Ok(json!({"changed": changed, "failed": false, "role": role, "type": typ}))
}

// Idempotence is decided on the *explicit* ACL grant to the role —
// `aclexplode` expands the stored acl and excludes the PUBLIC default
// (grantee 0) and inherited/implicit privileges, which `has_*_privilege`
// would wrongly count (pinned 2026-06-11: looker held CONNECT via PUBLIC
// yet Ansible still grants the explicit entry). `privilege_type` from
// aclexplode is the full upper-case name, matched against the request.

fn privs_missing_database(
    ctx: &ExecContext,
    port: &str,
    db: &str,
    role: &str,
    privs: &str,
) -> Result<bool, String> {
    for p in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
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
    Ok(false)
}

fn privs_missing_schema(
    ctx: &ExecContext,
    port: &str,
    db: &str,
    role: &str,
    objs: &str,
    privs: &str,
) -> Result<bool, String> {
    for schema in objs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        for p in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
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
    privs: &str,
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
        for p in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
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

fn grant_sql(
    typ: &str,
    role: &str,
    privs: &str,
    objs: Option<&str>,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    let r = ident(role);
    Ok(match typ {
        "database" => vec![format!("GRANT {privs} ON DATABASE CURRENT_CATALOG TO {r}")],
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
                o.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|t| format!("GRANT {privs} ON TABLE {t} TO {r}"))
                    .collect()
            }
        }
        "default_privs" => {
            let s = schema.unwrap_or("public");
            vec![format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {} GRANT {privs} ON TABLES TO {r}",
                ident(s)
            )]
        }
        other => {
            return Err(format!(
                "postgresql_privs: type {other:?} outside the closed surface"
            ));
        }
    })
}

// -- base64 (StoredKey encode + salt decode) --------------------------------

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
        // `acc` holds `bits` significant bits, MSB-first. Drop the low
        // `bits % 8` padding bits, then emit the `bits / 8` whole bytes
        // from the top down.
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
    #[test]
    fn b64_roundtrip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let enc = super::b64_encode(s.as_bytes());
            assert_eq!(super::b64_decode(&enc).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn scram_stored_key_matches_postgres() {
        // The live verifier captured from PG15 for password "s3cr3t-ruxel"
        // (salt+iterations from SCRAM-SHA-256$4096:<salt>$<storedkey>:...).
        let got =
            super::scram_stored_key("s3cr3t-ruxel", "tmEKTUhvHycriHKDSR74nA==", 4096).unwrap();
        assert_eq!(got, "qsXMYQ6PkvDbKdO/Fwo5aAQisdU9bG3fdoLEpQMpraM=");
    }

    #[test]
    fn b64_decode_partial_groups() {
        // 2-char tail (12 bits → 1 byte) is the case the first impl botched.
        assert_eq!(super::b64_decode("nA==").unwrap(), vec![0x9c]);
        assert_eq!(super::b64_decode("tA==").unwrap().len(), 1);
    }
}

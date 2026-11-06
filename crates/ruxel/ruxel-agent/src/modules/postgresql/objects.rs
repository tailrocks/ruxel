use super::{
    ExecContext, PgContext, Value, b64_decode, b64_encode, ident, json, lit, login_port,
    params_object, psql, role_flag_tokens, str_param, validate_role_attr_flags,
};

pub(crate) fn db(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let ctx = &PgContext {
        exec: ctx,
        login_user: str_param(obj, "login_user"),
        login_db: str_param(obj, "login_db"),
    };
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

pub(crate) fn user(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let ctx = &PgContext {
        exec: ctx,
        login_user: str_param(obj, "login_user"),
        login_db: str_param(obj, "login_db"),
    };
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
    ctx: &PgContext<'_>,
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
    if stored.starts_with("md5") {
        use md5::{Digest as _, Md5};
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut hash = Md5::new();
        hash.update(password.as_bytes());
        hash.update(name.as_bytes());
        let mut expected = String::from("md5");
        for byte in hash.finalize() {
            expected.push(char::from(HEX[usize::from(byte >> 4)]));
            expected.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        return Ok(stored != expected);
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

pub(super) fn scram_stored_key(
    password: &str,
    b64salt: &str,
    iterations: u32,
) -> Result<String, String> {
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

fn flags_changed(ctx: &PgContext<'_>, port: &str, name: &str, flags: &str) -> Result<bool, String> {
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

pub(super) fn wanted_flag(flags: &str, positive: &str) -> Option<bool> {
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

pub(crate) fn schema(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let ctx = &PgContext {
        exec: ctx,
        login_user: str_param(obj, "login_user"),
        login_db: str_param(obj, "login_db"),
    };
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

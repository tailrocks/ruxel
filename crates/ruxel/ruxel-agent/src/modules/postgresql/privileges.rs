use super::{ExecContext, Value, acl_privileges, ident, json, lit, login_port, params_object, psql, str_param, validate_privs};

pub(crate) fn privs(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
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

pub(super) fn default_priv_query(
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

pub(super) fn grant_sql(
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

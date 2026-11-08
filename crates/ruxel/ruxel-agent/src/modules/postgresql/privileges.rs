use super::{
    ExecContext, PgContext, Value, acl_privileges, ident, json, lit, login_port, params_object,
    psql, str_param, validate_privs,
};

fn text_list(
    obj: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(
            value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
        )),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| format!("postgresql_privs: {field} must contain text"))
                    .and_then(|text| {
                        if text.trim().is_empty() {
                            Err(format!("postgresql_privs: {field} contains an empty item"))
                        } else {
                            Ok(text.to_owned())
                        }
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Err(format!(
            "postgresql_privs: {field} must be text or a list of text"
        )),
    }
}

fn validate_state(state: &str) -> Result<(), String> {
    if state == "present" {
        Ok(())
    } else {
        Err(format!(
            "postgresql_privs: state {state:?} outside the closed surface"
        ))
    }
}

fn validate_targets(
    typ: &str,
    privs: Option<&[String]>,
    objs: Option<&[String]>,
    target_roles: Option<&[String]>,
) -> Result<(), String> {
    if privs.is_none_or(<[String]>::is_empty) {
        return Err("postgresql_privs: at least one privilege is required".into());
    }
    if matches!(typ, "database" | "schema" | "table") && objs.is_none_or(<[String]>::is_empty) {
        return Err("postgresql_privs: at least one object is required".into());
    }
    if target_roles.is_some_and(<[String]>::is_empty) {
        return Err("postgresql_privs: target_roles must not be empty".into());
    }
    Ok(())
}

pub(crate) fn privs(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let ctx = &PgContext {
        exec: ctx,
        login_user: str_param(obj, "login_user"),
        login_db: str_param(obj, "login_db"),
    };
    let login_db = str_param(obj, "login_db").ok_or("postgresql_privs: login_db required")?;
    let role = str_param(obj, "role").ok_or("postgresql_privs: role required")?;
    let typ = str_param(obj, "type").unwrap_or("table");
    let privs_values = text_list(obj, "privs")?;
    let privs_list = privs_values.as_deref().unwrap_or(&[]).join(",");
    let state = str_param(obj, "state").unwrap_or("present");
    let objs_values = text_list(obj, "objs")?;
    let objs = objs_values.as_deref();
    let schema = str_param(obj, "schema");
    let target_values = text_list(obj, "target_roles")?;
    let target_roles = target_values.as_deref();
    let port = login_port(obj);
    validate_state(state)?;
    validate_targets(typ, privs_values.as_deref(), objs, target_roles)?;
    validate_privs(&privs_list, typ)?;
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
    let probes = acl_privileges(&privs_list, typ, server_version);

    let needed = match typ {
        "database" => privs_missing_database(
            ctx,
            &port,
            objs.ok_or("postgresql_privs: objs required for type=database")?,
            role,
            &probes,
        )?,
        "schema" => privs_missing_schema(ctx, &port, login_db, role, objs.unwrap_or(&[]), &probes)?,
        "table" => privs_missing_table(
            ctx,
            &port,
            login_db,
            role,
            objs.unwrap_or(&[]),
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
            for sql in grant_sql(typ, role, &privs_list, objs, schema, target_roles)? {
                psql(ctx, &port, Some(login_db), &sql)?;
            }
        }
    }
    Ok(json!({"changed": changed, "failed": false, "role": role, "type": typ}))
}

fn privs_missing_database(
    ctx: &PgContext<'_>,
    port: &str,
    objs: &[String],
    role: &str,
    privs: &[String],
) -> Result<bool, String> {
    for db in objs {
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
    ctx: &PgContext<'_>,
    port: &str,
    db: &str,
    role: &str,
    objs: &[String],
    privs: &[String],
) -> Result<bool, String> {
    for schema in objs {
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
    ctx: &PgContext<'_>,
    port: &str,
    db: &str,
    role: &str,
    objs: &[String],
    schema: Option<&str>,
    privs: &[String],
) -> Result<bool, String> {
    let role_oid_sql = format!("(SELECT oid FROM pg_roles WHERE rolname={})", lit(role));
    let tables: Vec<(String, String)> = if objs.len() == 1 && objs[0] == "ALL_IN_SCHEMA" {
        let s = schema.ok_or("postgresql_privs: schema required for ALL_IN_SCHEMA")?;
        let rows = psql(
            ctx,
            port,
            Some(db),
            &format!(
                "SELECT COALESCE(json_agg(json_build_array(schemaname, tablename)), '[]'::json)::text FROM pg_tables WHERE schemaname={}",
                lit(s)
            ),
        )?;
        parse_table_rows(&rows)?
    } else {
        let s = schema.unwrap_or("public");
        objs.iter().map(|t| (s.to_string(), t.clone())).collect()
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

pub(super) fn parse_table_rows(rows: &str) -> Result<Vec<(String, String)>, String> {
    serde_json::from_str(rows)
        .map_err(|error| format!("postgresql_privs: invalid table rows: {error}"))
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
    ctx: &PgContext<'_>,
    port: &str,
    db: &str,
    role: &str,
    schema: &str,
    privs: &[String],
    target_roles: Option<&[String]>,
) -> Result<bool, String> {
    let targets: Vec<Option<&str>> = match target_roles {
        Some(roles) => roles.iter().map(|role| Some(role.as_str())).collect(),
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
    objs: Option<&[String]>,
    schema: Option<&str>,
    target_roles: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let r = ident(role);
    Ok(match typ {
        "database" => objs
            .ok_or("postgresql_privs: objs required for type=database")?
            .iter()
            .map(|db| format!("GRANT {privs} ON DATABASE {} TO {r}", ident(db)))
            .collect(),
        "schema" => objs
            .unwrap_or(&[])
            .iter()
            .map(|s| format!("GRANT {privs} ON SCHEMA {} TO {r}", ident(s)))
            .collect(),
        "table" => {
            let o = objs.unwrap_or(&[]);
            if o.len() == 1 && o[0] == "ALL_IN_SCHEMA" {
                let s = schema.ok_or("postgresql_privs: schema required")?;
                vec![format!(
                    "GRANT {privs} ON ALL TABLES IN SCHEMA {} TO {r}",
                    ident(s)
                )]
            } else {
                let s = schema.unwrap_or("public");
                o.iter()
                    .map(|t| format!("GRANT {privs} ON TABLE {}.{} TO {r}", ident(s), ident(t)))
                    .collect()
            }
        }
        "default_privs" => {
            let s = schema.unwrap_or("public");
            match target_roles {
                Some(roles) => roles.iter()
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

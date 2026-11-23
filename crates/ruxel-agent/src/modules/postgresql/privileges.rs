use super::{
    ExecContext, PgContext, Value, acl_privileges, ident, json, lit, login_port, params_object,
    psql, str_param, validate_privs,
};

mod defaults;
mod grants;
mod objects;
mod validation;

#[cfg(test)]
pub(super) use defaults::default_priv_query;
use defaults::privs_missing_default;
pub(super) use grants::grant_sql;
#[cfg(test)]
pub(super) use objects::parse_table_rows;
use objects::{privs_missing_database, privs_missing_schema, privs_missing_table};
use validation::{privilege_server_version, text_list, validate_state, validate_targets};

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

    let server_version = privilege_server_version(ctx, &port, login_db, typ, &privs_list)?;
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

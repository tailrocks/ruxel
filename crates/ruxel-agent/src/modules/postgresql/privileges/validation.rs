use super::{PgContext, Value, psql};

pub(super) fn text_list(
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

pub(super) fn validate_state(state: &str) -> Result<(), String> {
    if state == "present" {
        Ok(())
    } else {
        Err(format!(
            "postgresql_privs: state {state:?} outside the closed surface"
        ))
    }
}

pub(super) fn validate_targets(
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

pub(super) fn privilege_server_version(
    ctx: &PgContext<'_>,
    port: &str,
    db: &str,
    typ: &str,
    privs: &str,
) -> Result<u32, String> {
    let has_maintain = privs
        .split(',')
        .map(str::trim)
        .any(|p| p.eq_ignore_ascii_case("MAINTAIN"));
    let needs_version = matches!(typ, "table" | "default_privs")
        && (has_maintain
            || privs.split(',').map(str::trim).any(|p| {
                p.eq_ignore_ascii_case("ALL") || p.eq_ignore_ascii_case("ALL PRIVILEGES")
            }));
    if !needs_version {
        return Ok(0);
    }
    let version = psql(ctx, port, Some(db), "SHOW server_version_num")?
        .parse::<u32>()
        .map_err(|_| "postgresql_privs: invalid server_version_num")?;
    if has_maintain && version < 170_000 {
        return Err("postgresql_privs: MAINTAIN requires PostgreSQL 17 or newer".into());
    }
    Ok(version)
}

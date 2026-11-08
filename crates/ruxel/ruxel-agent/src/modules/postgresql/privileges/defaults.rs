use super::{PgContext, lit, psql};

pub(crate) fn default_priv_query(
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

pub(super) fn privs_missing_default(
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

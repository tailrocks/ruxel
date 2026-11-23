use super::ident;

pub(crate) fn grant_sql(
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

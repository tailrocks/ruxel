use super::{PgContext, lit, psql};

pub(super) fn privs_missing_database(
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

pub(super) fn privs_missing_schema(
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

pub(super) fn privs_missing_table(
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

pub(crate) fn parse_table_rows(rows: &str) -> Result<Vec<(String, String)>, String> {
    serde_json::from_str(rows)
        .map_err(|error| format!("postgresql_privs: invalid table rows: {error}"))
}

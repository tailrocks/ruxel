use crate::host_fixture::Guest;
use anyhow::{Result, ensure};

const SNAPSHOT: &str = "SELECT json_build_object('roles', (SELECT json_agg(r ORDER BY rolname) FROM (SELECT rolname,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolinherit,rolreplication,rolbypassrls,rolvaliduntil,rolconnlimit,rolpassword FROM pg_authid) r), 'databases', (SELECT json_agg(r ORDER BY datname) FROM (SELECT datname,datdba::regrole::text,datacl FROM pg_database) r), 'schemas', (SELECT json_agg(r ORDER BY nspname) FROM (SELECT nspname,nspowner::regrole::text,nspacl FROM pg_namespace) r), 'defaults', (SELECT json_agg(r ORDER BY role_name,schema_name,defaclobjtype) FROM (SELECT defaclrole::regrole::text AS role_name,defaclnamespace::regnamespace::text AS schema_name,defaclobjtype,defaclacl FROM pg_default_acl) r))::text";
const OBSERVE: &str = "SELECT (SELECT rolcanlogin FROM pg_roles WHERE rolname='ruxel_fixture_owner') IS TRUE AND EXISTS(SELECT 1 FROM pg_roles WHERE rolname='ruxel_fixture_reader') AND (SELECT datdba::regrole::text FROM pg_database WHERE datname='ruxel_fixture_database')='ruxel_fixture_owner' AND (SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname='ruxel_fixture_schema')='ruxel_fixture_owner' AND EXISTS(SELECT 1 FROM pg_database d CROSS JOIN LATERAL aclexplode(d.datacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE d.datname='ruxel_fixture_database' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='CONNECT') AND EXISTS(SELECT 1 FROM pg_namespace n CROSS JOIN LATERAL aclexplode(n.nspacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE n.nspname='ruxel_fixture_schema' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='USAGE') AND EXISTS (SELECT 1 FROM pg_default_acl d JOIN pg_roles owner ON owner.oid=d.defaclrole JOIN pg_namespace n ON n.oid=d.defaclnamespace CROSS JOIN LATERAL aclexplode(d.defaclacl) a JOIN pg_roles reader ON reader.oid=a.grantee WHERE owner.rolname='ruxel_fixture_owner' AND n.nspname='ruxel_fixture_schema' AND d.defaclobjtype='r' AND reader.rolname='ruxel_fixture_reader' AND a.privilege_type='SELECT') AND NOT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='ruxel_fixture_literal_schema')";
const DRIFT: &str = "ALTER ROLE ruxel_fixture_owner NOLOGIN PASSWORD 'synthetic-drift-password'; ALTER DATABASE ruxel_fixture_database OWNER TO postgres; ALTER SCHEMA ruxel_fixture_schema OWNER TO postgres; REVOKE CONNECT ON DATABASE ruxel_fixture_database FROM ruxel_fixture_reader; REVOKE USAGE ON SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader; REVOKE SELECT ON ALL TABLES IN SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader; ALTER DEFAULT PRIVILEGES FOR ROLE ruxel_fixture_owner IN SCHEMA ruxel_fixture_schema REVOKE SELECT ON TABLES FROM ruxel_fixture_reader";
const TABLE_OBSERVE: &str = "SELECT count(*)=2 AND bool_and(has_table_privilege('ruxel_fixture_reader',c.oid,'SELECT')) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='ruxel_fixture_schema' AND c.relname IN ('explicit_table','default_table') AND c.relkind='r'";
const TABLE_SNAPSHOT: &str = "SELECT json_agg(r ORDER BY relname)::text FROM (SELECT c.relname,c.relowner,c.relacl FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='ruxel_fixture_schema') r";
const TASKS: [&str; 9] = [
    "Create fixture owner",
    "Create fixture reader",
    "Create fixture database",
    "Create owned fixture schema",
    "Exercise literal PostgreSQL schema names",
    "Grant fixture default table reads",
    "Grant fixture database access",
    "Grant fixture schema access",
    "Grant fixture table access shape",
];

fn changes(events: &[serde_json::Value], changed: &[&str]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for row in events.iter().filter(|e| e.get("result").is_some()) {
        let task = row["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing task name"))?;
        ensure!(
            TASKS.contains(&task) && seen.insert(task),
            "unknown or duplicate PostgreSQL result: {task}"
        );
        ensure!(
            row["result"]["changed"] == changed.contains(&task),
            "unexpected PostgreSQL change prediction for {task}"
        );
    }
    ensure!(seen.len() == TASKS.len(), "missing PostgreSQL results");
    Ok(())
}

fn exercise_tables(guest: &Guest) -> Result<()> {
    query(
        guest,
        "CREATE TABLE ruxel_fixture_schema.explicit_table(id integer); SET ROLE ruxel_fixture_owner; CREATE TABLE ruxel_fixture_schema.default_table(id integer); RESET ROLE",
    )?;
    ensure!(query(guest, "SELECT NOT has_table_privilege('ruxel_fixture_reader','ruxel_fixture_schema.explicit_table','SELECT') AND has_table_privilege('ruxel_fixture_reader','ruxel_fixture_schema.default_table','SELECT')")?.trim() == "t", "creator-specific default privileges did not apply to the new table");
    changes(&guest.run("apply", 0)?, &[TASKS[4], TASKS[8]])?;
    ensure!(
        query(guest, TABLE_OBSERVE)?.trim() == "t",
        "explicit table grant missing"
    );
    ensure!(query(guest, &format!("BEGIN; REVOKE SELECT ON ruxel_fixture_schema.explicit_table FROM ruxel_fixture_reader; {TABLE_OBSERVE}; ROLLBACK"))?.trim() == "f", "table observation missed revoked grant");
    ensure!(query(guest, "SELECT rolpassword LIKE 'SCRAM-SHA-256$%' FROM pg_authid WHERE rolname='ruxel_fixture_owner'")?.trim() == "t", "fixture did not exercise SCRAM password convergence");
    Ok(())
}

fn authenticate(guest: &Guest, password: &str, expected: i32) -> Result<()> {
    let output = guest.docker.exec(
        &[
            &format!("PGPASSWORD={password}"),
            "/usr/bin/psql",
            "-X",
            "-qAt",
            "-w",
            "-h",
            "/tmp/work/pg/socket",
            "-p",
            "40000",
            "-U",
            "ruxel_fixture_owner",
            "-d",
            "postgres",
            "-c",
            "SELECT 1",
        ],
        expected,
    )?;
    ensure!(
        expected != 0 || output == b"1\n",
        "password authentication observation failed"
    );
    Ok(())
}

fn password_repaired(guest: &Guest) -> Result<()> {
    authenticate(guest, "synthetic-fixture-password", 0)?;
    authenticate(guest, "synthetic-drift-password", 2)
}

fn exercise_password(guest: &Guest) -> Result<()> {
    password_repaired(guest)?;
    query(
        guest,
        "ALTER ROLE ruxel_fixture_owner PASSWORD 'synthetic-drift-password'",
    )?;
    ensure!(
        query(
            guest,
            "SELECT rolcanlogin FROM pg_roles WHERE rolname='ruxel_fixture_owner'"
        )?
        .trim()
            == "t",
        "password-only drift changed LOGIN"
    );
    authenticate(guest, "synthetic-fixture-password", 2)?;
    authenticate(guest, "synthetic-drift-password", 0)?;
    let before = query(guest, SNAPSHOT)?;
    let files = guest.snapshot()?;
    changes(&guest.run("check", 0)?, &[TASKS[0], TASKS[4]])?;
    ensure!(
        query(guest, SNAPSHOT)? == before && guest.snapshot()? == files,
        "password-only check mutated state"
    );
    changes(&guest.run("apply", 0)?, &[TASKS[0], TASKS[4]])?;
    password_repaired(guest)?;
    changes(&guest.run("apply", 0)?, &[TASKS[4]])?;
    println!("PASS postgresql-ownership password-only drift, SCRAM authentication and convergence");
    Ok(())
}

fn query(guest: &Guest, sql: &str) -> Result<String> {
    guest.text(&[
        "/usr/sbin/runuser",
        "-u",
        "postgres",
        "--",
        "/usr/bin/psql",
        "-X",
        "-qAt",
        "-v",
        "ON_ERROR_STOP=1",
        "-h",
        "/tmp/work/pg/socket",
        "-p",
        "40000",
        "-U",
        "postgres",
        "-d",
        "postgres",
        "-c",
        sql,
    ])
}
fn start(guest: &Guest) -> Result<()> {
    let bindir = guest.text(&["/usr/bin/pg_config", "--bindir"])?;
    let bindir = bindir.trim();
    ensure!(
        bindir.starts_with("/usr/lib/postgresql/")
            && bindir
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)),
        "unexpected postgres bindir"
    );
    guest.docker.exec(
        &[
            "/bin/mkdir",
            "-p",
            "/tmp/work/pg/data",
            "/tmp/work/pg/socket",
        ],
        0,
    )?;
    guest.docker.exec(
        &["/usr/bin/chown", "-R", "postgres:postgres", "/tmp/work/pg"],
        0,
    )?;
    guest.docker.exec(
        &[
            "/usr/sbin/runuser",
            "-u",
            "postgres",
            "--",
            &format!("{bindir}/initdb"),
            "-D",
            "/tmp/work/pg/data",
            "--auth-local=trust",
            "--no-instructions",
            "--no-locale",
        ],
        0,
    )?;
    guest.docker.exec(&[
        "/bin/bash", "-euo", "pipefail", "-c",
        "printf '%s\\n' 'local all ruxel_fixture_owner scram-sha-256' 'local all all trust' > /tmp/work/pg/data/pg_hba.conf",
    ], 0)?;
    guest.docker.exec(
        &[
            "/usr/sbin/runuser",
            "-u",
            "postgres",
            "--",
            &format!("{bindir}/pg_ctl"),
            "-D",
            "/tmp/work/pg/data",
            "-l",
            "/tmp/work/pg/postgres.log",
            "-w",
            "-t",
            "20",
            "-o",
            "-k /tmp/work/pg/socket -p 40000 -h ''",
            "start",
        ],
        0,
    )?;
    Ok(())
}
pub(super) fn execute(guest: &Guest) -> Result<()> {
    start(guest)?;
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if matches!(phase, "drift_repair" | "check") {
            let before = query(guest, SNAPSHOT)?;
            query(guest, DRIFT)?;
            ensure!(
                query(guest, SNAPSHOT)? != before,
                "postgres drift was not applied"
            );
        }
        let preview = matches!(phase, "check" | "diff");
        let before = query(guest, SNAPSHOT)?;
        let table_before = query(guest, TABLE_SNAPSHOT)?;
        let files = guest.snapshot()?;
        let events = guest.run(if preview { phase } else { "apply" }, 0)?;
        if phase == "converged" {
            changes(&events, &[TASKS[4]])?;
        }
        if preview {
            let predicted: Vec<_> = TASKS.into_iter().filter(|task| *task != TASKS[1]).collect();
            changes(&events, &predicted)?;
            ensure!(
                query(guest, SNAPSHOT)? == before
                    && query(guest, TABLE_SNAPSHOT)? == table_before
                    && guest.snapshot()? == files,
                "postgres preview mutated state"
            );
        } else {
            ensure!(
                query(guest, OBSERVE)?.trim() == "t",
                "postgres observations mismatch"
            );
            if phase == "fresh" {
                for revoke in [
                    "REVOKE CONNECT ON DATABASE ruxel_fixture_database FROM ruxel_fixture_reader",
                    "REVOKE USAGE ON SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader",
                    "ALTER DEFAULT PRIVILEGES FOR ROLE ruxel_fixture_owner IN SCHEMA ruxel_fixture_schema REVOKE SELECT ON TABLES FROM ruxel_fixture_reader",
                ] {
                    ensure!(
                        query(guest, &format!("BEGIN; {revoke}; {OBSERVE}; ROLLBACK"))?.trim()
                            == "f",
                        "postgres observation missed revoked grant"
                    );
                }
                exercise_tables(guest)?;
                exercise_password(guest)?;
            } else {
                ensure!(
                    query(guest, TABLE_OBSERVE)?.trim() == "t",
                    "table grant repair missing"
                );
                password_repaired(guest)?;
            }
        }
        println!("PASS postgresql-ownership {phase}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predictions_require_every_unique_task_and_exact_changed_flags() {
        let mut events: Vec<_> = TASKS
            .iter()
            .map(|task| {
                serde_json::json!({
                    "task": task, "result": {"changed": *task != TASKS[1]}
                })
            })
            .collect();
        let predicted: Vec<_> = TASKS.into_iter().filter(|task| *task != TASKS[1]).collect();
        assert!(changes(&events, &predicted).is_ok());
        events[0]["result"]["changed"] = false.into();
        assert!(changes(&events, &predicted).is_err());
        events[0]["result"]["changed"] = true.into();
        events[0]["task"] = TASKS[1].into();
        assert!(changes(&events, &predicted).is_err());
        assert!(changes(&events[..8], &predicted).is_err());
    }
}

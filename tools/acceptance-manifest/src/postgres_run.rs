use crate::host_fixture::Guest;
use anyhow::{Result, ensure};

const SNAPSHOT: &str = "SELECT json_build_object('roles', (SELECT json_agg(r ORDER BY rolname) FROM (SELECT rolname,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolinherit,rolreplication,rolbypassrls,rolvaliduntil,rolconnlimit,rolpassword FROM pg_authid) r), 'databases', (SELECT json_agg(r ORDER BY datname) FROM (SELECT datname,datdba::regrole::text,datacl FROM pg_database) r), 'schemas', (SELECT json_agg(r ORDER BY nspname) FROM (SELECT nspname,nspowner::regrole::text,nspacl FROM pg_namespace) r), 'defaults', (SELECT json_agg(r ORDER BY role_name,schema_name,defaclobjtype) FROM (SELECT defaclrole::regrole::text AS role_name,defaclnamespace::regnamespace::text AS schema_name,defaclobjtype,defaclacl FROM pg_default_acl) r))::text";
const OBSERVE: &str = "SELECT (SELECT rolcanlogin FROM pg_roles WHERE rolname='ruxel_fixture_owner') IS TRUE AND EXISTS(SELECT 1 FROM pg_roles WHERE rolname='ruxel_fixture_reader') AND (SELECT datdba::regrole::text FROM pg_database WHERE datname='ruxel_fixture_database')='ruxel_fixture_owner' AND (SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname='ruxel_fixture_schema')='ruxel_fixture_owner' AND EXISTS(SELECT 1 FROM pg_database d CROSS JOIN LATERAL aclexplode(d.datacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE d.datname='ruxel_fixture_database' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='CONNECT') AND EXISTS(SELECT 1 FROM pg_namespace n CROSS JOIN LATERAL aclexplode(n.nspacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE n.nspname='ruxel_fixture_schema' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='USAGE') AND EXISTS (SELECT 1 FROM pg_default_acl d JOIN pg_roles owner ON owner.oid=d.defaclrole JOIN pg_namespace n ON n.oid=d.defaclnamespace CROSS JOIN LATERAL aclexplode(d.defaclacl) a JOIN pg_roles reader ON reader.oid=a.grantee WHERE owner.rolname='ruxel_fixture_owner' AND n.nspname='ruxel_fixture_schema' AND d.defaclobjtype='r' AND reader.rolname='ruxel_fixture_reader' AND a.privilege_type='SELECT') AND NOT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='ruxel_fixture_literal_schema')";
const DRIFT: &str = "ALTER SCHEMA ruxel_fixture_schema OWNER TO postgres; REVOKE USAGE ON SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader";

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
        let files = guest.snapshot()?;
        let events = guest.run(if preview { phase } else { "apply" }, 0)?;
        if phase == "converged" {
            let rows: Vec<_> = events
                .iter()
                .filter(|e| e.get("result").is_some())
                .collect();
            ensure!(rows.len() == 9, "missing converged PostgreSQL results");
            for row in rows {
                let prediction = row["task"] == "Exercise literal PostgreSQL schema names";
                ensure!(
                    row["result"]["changed"] == prediction,
                    "unexpected converged change for {}",
                    row["task"]
                );
            }
        }
        if preview {
            ensure!(
                query(guest, SNAPSHOT)? == before && guest.snapshot()? == files,
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
            }
        }
        println!("PASS postgresql-ownership {phase}");
    }
    Ok(())
}

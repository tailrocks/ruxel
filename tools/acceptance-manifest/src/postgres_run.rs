use crate::{
    postgres_cluster::{self, Cluster},
    process,
    runner::Env,
    state,
};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    process::{Command, Stdio},
    time::Duration,
};

const SNAPSHOT: &str = "SELECT json_build_object('roles', (SELECT json_agg(r ORDER BY rolname) FROM (SELECT rolname,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolinherit,rolreplication,rolbypassrls,rolvaliduntil,rolconnlimit,rolpassword FROM pg_authid) r), 'databases', (SELECT json_agg(r ORDER BY datname) FROM (SELECT datname,datdba::regrole::text,datacl FROM pg_database) r), 'schemas', (SELECT json_agg(r ORDER BY nspname) FROM (SELECT nspname,nspowner::regrole::text,nspacl FROM pg_namespace) r), 'defaults', (SELECT json_agg(r ORDER BY role_name,schema_name,defaclobjtype) FROM (SELECT defaclrole::regrole::text AS role_name,defaclnamespace::regnamespace::text AS schema_name,defaclobjtype,defaclacl FROM pg_default_acl) r))::text";
const OBSERVE: &str = "SELECT (SELECT rolcanlogin FROM pg_roles WHERE rolname='ruxel_fixture_owner') IS TRUE AND EXISTS(SELECT 1 FROM pg_roles WHERE rolname='ruxel_fixture_reader') AND (SELECT datdba::regrole::text FROM pg_database WHERE datname='ruxel_fixture_database')='ruxel_fixture_owner' AND (SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname='ruxel_fixture_schema')='ruxel_fixture_owner' AND EXISTS(SELECT 1 FROM pg_database d CROSS JOIN LATERAL aclexplode(d.datacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE d.datname='ruxel_fixture_database' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='CONNECT') AND EXISTS(SELECT 1 FROM pg_namespace n CROSS JOIN LATERAL aclexplode(n.nspacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE n.nspname='ruxel_fixture_schema' AND r.rolname='ruxel_fixture_reader' AND a.privilege_type='USAGE') AND EXISTS (SELECT 1 FROM pg_default_acl d JOIN pg_roles owner ON owner.oid=d.defaclrole JOIN pg_namespace n ON n.oid=d.defaclnamespace CROSS JOIN LATERAL aclexplode(d.defaclacl) a JOIN pg_roles reader ON reader.oid=a.grantee WHERE owner.rolname='ruxel_fixture_owner' AND n.nspname='ruxel_fixture_schema' AND d.defaclobjtype='r' AND reader.rolname='ruxel_fixture_reader' AND a.privilege_type='SELECT') AND NOT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='ruxel_fixture_literal_schema')";
const DRIFT: &str = "ALTER SCHEMA ruxel_fixture_schema OWNER TO postgres; REVOKE USAGE ON SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader";

fn run_cli(env: &Env, cluster: &Cluster, mode: &str) -> Result<()> {
    let mut command = Command::new(&env.cli);
    postgres_cluster::clear_pg(&mut command);
    command
        .env("PGHOST", &cluster.socket)
        .arg(mode)
        .arg(&env.module)
        .args(["--local", "--agent"])
        .arg(&env.agent)
        .arg("--state-dir")
        .arg(env.root.path().join("state"))
        .stdin(Stdio::null());
    let (status, out, _) = process::run(&mut command, Duration::from_secs(60))?;
    ensure!(status.success(), "postgres CLI phase failed");
    let events: Vec<Value> = std::str::from_utf8(&out)?
        .lines()
        .filter(|v| !v.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let recaps: Vec<_> = events.iter().filter(|v| v.get("recap").is_some()).collect();
    ensure!(
        recaps.len() == 1 && recaps[0]["recap"]["failed"] == 0,
        "postgres CLI recap mismatch"
    );
    Ok(())
}
pub(super) fn execute(env: &Env) -> Result<()> {
    let mut cluster = Cluster::start(env.root.path())?;
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if matches!(phase, "drift_repair" | "check") {
            let before = cluster.query(SNAPSHOT)?;
            cluster.query(DRIFT)?;
            ensure!(
                cluster.query(SNAPSHOT)? != before,
                "postgres drift was not applied"
            );
        }
        let preview = matches!(phase, "check" | "diff");
        let db_before = cluster.query(SNAPSHOT)?;
        let payload_before = state::snapshot(&env.payload)?;
        let schema_before = state::snapshot(&env.root.path().join("schema"))?;
        let fixture_before = state::snapshot(&env.root.path().join("testdata"))?;
        let mode = if preview { phase } else { "apply" };
        run_cli(env, &cluster, mode)?;
        if preview {
            ensure!(
                cluster.query(SNAPSHOT)? == db_before
                    && state::snapshot(&env.payload)? == payload_before
                    && state::snapshot(&env.root.path().join("schema"))? == schema_before
                    && state::snapshot(&env.root.path().join("testdata"))? == fixture_before,
                "postgres preview mutated state"
            );
        } else {
            ensure!(
                cluster.query(OBSERVE)?.trim() == "t",
                "postgres observations mismatch"
            );
            if phase == "fresh" {
                for revoke in [
                    "REVOKE CONNECT ON DATABASE ruxel_fixture_database FROM ruxel_fixture_reader",
                    "REVOKE USAGE ON SCHEMA ruxel_fixture_schema FROM ruxel_fixture_reader",
                    "ALTER DEFAULT PRIVILEGES FOR ROLE ruxel_fixture_owner IN SCHEMA ruxel_fixture_schema REVOKE SELECT ON TABLES FROM ruxel_fixture_reader",
                ] {
                    ensure!(
                        cluster
                            .query(&format!("BEGIN; {revoke}; {OBSERVE}; ROLLBACK"))?
                            .trim()
                            == "f",
                        "postgres observation did not detect a missing grant"
                    );
                }
            }
        }
        println!("PASS postgresql-ownership {phase}");
    }
    cluster.stop()
}

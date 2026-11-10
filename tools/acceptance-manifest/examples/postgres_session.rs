use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{env, fs, path::Path};

#[path = "../src/docker_fixture.rs"]
mod docker_fixture;
#[allow(dead_code)]
#[path = "../src/pins.rs"]
mod pins;
#[path = "../src/process.rs"]
mod process;
#[path = "../src/process_guard.rs"]
mod process_guard;

use docker_fixture::Docker;
const FIXTURE: &str = r#"
import "modulepath:/ruxel/engine/Engine.pkl" as Ruxel
output { value = new Ruxel.Config {
  inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
  plays { new { name = "Synthetic PostgreSQL session reuse"; targets { "all" }; tasks {
    new { name = "First synthetic role"; becomeUser = "postgres"; operation = new Ruxel.PostgresRole { login_user = "postgres"; login_db = "postgres"; name = "ruxel_session_a"; state = "present"; login_port = 40000 } }
    new { name = "Second synthetic role"; becomeUser = "postgres"; operation = new Ruxel.PostgresRole { login_user = "__SECOND_USER__"; login_db = "postgres"; name = "ruxel_session_b"; state = "present"; login_port = 40000 } }
  } } }
} }
"#;
const SNAPSHOT: &str = "SELECT rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit FROM pg_roles WHERE rolname IN ('ruxel_session_a','ruxel_session_b','ruxel_session_alt') ORDER BY rolname";

fn text(docker: &Docker, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(docker.exec(args, 0)?)?)
}
fn query(docker: &Docker, sql: &str) -> Result<String> {
    text(
        docker,
        &[
            "/usr/sbin/runuser",
            "-u",
            "postgres",
            "--",
            "/usr/bin/psql",
            "-XqAt",
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
        ],
    )
}
fn start(docker: &Docker) -> Result<()> {
    let bindir = text(docker, &["/usr/bin/pg_config", "--bindir"])?;
    let bindir = bindir.trim();
    ensure!(
        bindir.starts_with("/usr/lib/postgresql/")
            && bindir
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)),
        "invalid PostgreSQL binary path"
    );
    docker.exec(
        &[
            "/bin/mkdir",
            "-p",
            "/tmp/work/pg/data",
            "/tmp/work/pg/socket",
            "/opt/ruxel-fixture",
        ],
        0,
    )?;
    docker.exec(
        &["/usr/bin/chown", "-R", "postgres:postgres", "/tmp/work/pg"],
        0,
    )?;
    docker.exec(
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
    docker.exec(
        &[
            "/usr/sbin/runuser",
            "-u",
            "postgres",
            "--",
            &format!("{bindir}/pg_ctl"),
            "-D",
            "/tmp/work/pg/data",
            "-l",
            "/tmp/work/pg/log",
            "-w",
            "-t",
            "20",
            "-o",
            "-k /tmp/work/pg/socket -p 40000 -h '' -c log_connections=on",
            "start",
        ],
        0,
    )?;
    query(
        docker,
        "CREATE ROLE ruxel_session_a; CREATE ROLE ruxel_session_b; CREATE ROLE ruxel_session_alt LOGIN SUPERUSER",
    )?;
    Ok(())
}
fn stage(docker: &Docker, cli: &Path, agent: &Path, temp: &Path) -> Result<()> {
    for (source, target) in [
        (cli, "/opt/ruxel-fixture/ruxel"),
        (agent, "/opt/ruxel-fixture/ruxel-agent"),
    ] {
        let bytes = fs::read(source)?;
        ensure!(bytes.starts_with(b"\x7fELF"), "Linux ELF binaries required");
        let staged = temp.join(target.rsplit('/').next().context("invalid stage")?);
        fs::write(&staged, bytes)?;
        docker.copy(&staged, target)?;
    }
    docker.exec(
        &[
            "/bin/chmod",
            "755",
            "/opt/ruxel-fixture/ruxel",
            "/opt/ruxel-fixture/ruxel-agent",
        ],
        0,
    )?;
    Ok(())
}
fn verify_connections(added: &str, mode: &str, second_user: &str, sessions: usize) -> Result<()> {
    let authorized: Vec<_> = added
        .lines()
        .filter(|line| line.contains("connection authorized:"))
        .collect();
    let observed = authorized.len();
    ensure!(
        observed == sessions,
        "{mode}: expected {sessions} real PostgreSQL sessions, saw {observed}"
    );
    for user in if second_user == "postgres" {
        vec!["postgres"]
    } else {
        vec!["postgres", second_user]
    } {
        ensure!(
            authorized
                .iter()
                .filter(|line| line.contains(&format!("user={user} ")))
                .count()
                == 1,
            "{mode}: expected exactly one authorized connection for {user}"
        );
    }
    Ok(())
}

fn phase(
    docker: &Docker,
    temp: &Path,
    mode: &str,
    second_user: &str,
    sessions: usize,
) -> Result<()> {
    let workload = temp.join("workload.pkl");
    fs::write(&workload, FIXTURE.replace("__SECOND_USER__", second_user))?;
    docker.copy(&workload, "/tmp/work/workload.pkl")?;
    let before = query(docker, SNAPSHOT)?;
    let log_before = text(docker, &["/bin/cat", "/tmp/work/pg/log"])?;
    let result = docker.exec(
        &[
            "PGHOST=/tmp/work/pg/socket",
            "/opt/ruxel-fixture/ruxel",
            mode,
            "/tmp/work/workload.pkl",
            "--local",
            "--agent",
            "/opt/ruxel-fixture/ruxel-agent",
            "--state-dir",
            "/tmp/work/state",
            "--no-cache",
        ],
        0,
    )?;
    let log_after = text(docker, &["/bin/cat", "/tmp/work/pg/log"])?;
    let added = log_after
        .strip_prefix(&log_before)
        .context("PostgreSQL log was unexpectedly replaced")?;
    verify_connections(added, mode, second_user, sessions)?;
    let events: Vec<Value> = std::str::from_utf8(&result)?
        .lines()
        .filter(|s| !s.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    for (task, role) in [
        ("First synthetic role", "ruxel_session_a"),
        ("Second synthetic role", "ruxel_session_b"),
    ] {
        let matching: Vec<_> = events
            .iter()
            .filter(|event| event["task"] == task)
            .collect();
        ensure!(
            matching.len() == 1
                && matching[0]["result"]["failed"] == false
                && (matching[0]["result"]["skipped"].is_null()
                    || matching[0]["result"]["skipped"] == false)
                && matching[0]["result"]["changed"] == false
                && matching[0]["result"]["user"] == role,
            "{mode}: {task} did not converge"
        );
    }
    ensure!(
        events
            .iter()
            .filter_map(|event| event.get("recap"))
            .any(|recap| recap["failed"] == 0),
        "{mode}: missing successful recap"
    );
    ensure!(
        before == query(docker, SNAPSHOT)?,
        "{mode}: observed role attributes changed"
    );
    println!(
        "{mode} ({second_user}): {sessions} PostgreSQL session(s), observed role attributes unchanged"
    );
    Ok(())
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let cli = fs::canonicalize(args.next().context("expected CLI path")?)?;
    let agent = fs::canonicalize(args.next().context("expected agent path")?)?;
    let image = args
        .next()
        .context("expected local image sha256")?
        .into_string()
        .map_err(|_| anyhow::anyhow!("invalid image ID"))?;
    ensure!(args.next().is_none(), "expected CLI, agent, and image ID");
    let temp = tempfile::tempdir()?;
    let mut docker = Docker::create(&image)?;
    start(&docker)?;
    stage(&docker, &cli, &agent, temp.path())?;
    for mode in ["check", "diff"] {
        phase(&docker, temp.path(), mode, "postgres", 1)?;
    }
    phase(&docker, temp.path(), "check", "ruxel_session_alt", 2)?;
    docker.close()?;
    Ok(())
}

#[cfg(test)]
mod proof_tests {
    use super::verify_connections;

    #[test]
    fn connection_count_requires_the_expected_identities() {
        let one = "LOG: connection authorized: user=postgres database=postgres\n";
        let two = "LOG: connection authorized: user=postgres database=postgres\nLOG: connection authorized: user=ruxel_session_alt database=postgres\n";
        assert!(verify_connections(one, "check", "postgres", 1).is_ok());
        assert!(verify_connections(two, "check", "ruxel_session_alt", 2).is_ok());
        assert!(verify_connections(one, "check", "ruxel_session_alt", 2).is_err());
        assert!(verify_connections(two, "check", "postgres", 1).is_err());
        assert!(verify_connections(two, "check", "other", 2).is_err());
    }
}

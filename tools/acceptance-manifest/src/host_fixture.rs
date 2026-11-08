use crate::docker_fixture::Docker;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

pub(super) struct Guest {
    pub docker: Docker,
    scenario: String,
}

fn regular_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())?)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "fixture input must be a regular file"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl Guest {
    pub fn new(scenario: &str, root: &Path, cli: &Path, agent: &Path, image: &str) -> Result<Self> {
        ensure!(
            ["postgresql-ownership", "become-environment"].contains(&scenario),
            "unsupported host fixture"
        );
        let data = regular_bytes(&root.join(format!("testdata/workloads/pr66/{scenario}.pkl")))?;
        ensure!(
            crate::pins::digest(&data) == crate::pins::pin(scenario),
            "fixture pin mismatch"
        );
        let stage = tempfile::Builder::new()
            .prefix("ruxel-fixture-input-")
            .tempdir()?;
        for (binary, name) in [(cli, "ruxel"), (agent, "ruxel-agent")] {
            let data = regular_bytes(binary)?;
            ensure!(
                data.starts_with(b"\x7fELF"),
                "isolated fixtures require Linux ELF binaries"
            );
            fs::write(stage.path().join(name), data)?;
        }
        let schema = stage.path().join("Ruxel.pkl");
        fs::write(&schema, regular_bytes(&root.join("schema/Ruxel.pkl"))?)?;
        let workload = stage.path().join("workload.pkl");
        fs::write(
            &workload,
            std::str::from_utf8(&data)?
                .replace("../../../schema/Ruxel.pkl", "schema/Ruxel.pkl")
                .replace("__RUN_ROOT__", "/tmp/work/payload"),
        )?;
        let docker = Docker::create(image)?;
        docker.exec(
            &[
                "/bin/mkdir",
                "-p",
                "/tmp/work/schema",
                "/tmp/work/payload",
                "/opt/ruxel-fixture",
            ],
            0,
        )?;
        docker.exec(&["/bin/chmod", "711", "/tmp/work", "/tmp/work/payload"], 0)?;
        docker.copy(&schema, "/tmp/work/schema/Ruxel.pkl")?;
        docker.copy(&workload, "/tmp/work/workload.pkl")?;
        docker.copy(&stage.path().join("ruxel"), "/opt/ruxel-fixture/ruxel")?;
        docker.copy(
            &stage.path().join("ruxel-agent"),
            "/opt/ruxel-fixture/ruxel-agent",
        )?;
        docker.exec(
            &[
                "/bin/chmod",
                "755",
                "/opt/ruxel-fixture/ruxel",
                "/opt/ruxel-fixture/ruxel-agent",
            ],
            0,
        )?;
        Ok(Self {
            docker,
            scenario: scenario.into(),
        })
    }

    pub fn run(&self, mode: &str, expected: i32) -> Result<Vec<Value>> {
        ensure!(
            ["apply", "check", "diff"].contains(&mode),
            "invalid fixture mode"
        );
        let args = [
            "/opt/ruxel-fixture/ruxel",
            mode,
            "/tmp/work/workload.pkl",
            "--local",
            "--agent",
            "/opt/ruxel-fixture/ruxel-agent",
            "--state-dir",
            "/tmp/work/state",
        ];
        let output = if self.scenario == "postgresql-ownership" {
            let mut pg = vec!["PGHOST=/tmp/work/pg/socket"];
            pg.extend(args);
            self.docker.exec(&pg, expected)?
        } else {
            self.docker.exec(&args, expected)?
        };
        let events = std::str::from_utf8(&output)?
            .lines()
            .filter(|s| !s.is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        let recaps: Vec<_> = events.iter().filter_map(|e| e.get("recap")).collect();
        ensure!(
            recaps.len() == 1 && recaps[0]["failed"] == if expected == 0 { 0 } else { 1 },
            "fixture recap mismatch"
        );
        Ok(events)
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        self.docker
            .exec(&["/bin/bash", "-euo", "pipefail", "-c", SNAPSHOT], 0)
    }
    pub fn text(&self, args: &[&str]) -> Result<String> {
        String::from_utf8(self.docker.exec(args, 0)?).context("fixture output encoding")
    }
}

const SNAPSHOT: &str = r#"find /etc /var/log /tmp/work/payload /tmp/work/schema /tmp/work/workload.pkl -print0 | sort -z | while IFS= read -r -d '' path; do
stat --printf='%n\0%f\0%u\0%g\0%i\0%s\0%y\0%z\0' -- "$path"
if test -L "$path"; then readlink -z -- "$path"; elif test -f "$path"; then sha256sum --zero -- "$path"; elif ! test -d "$path"; then exit 1; fi
done | sha256sum"#;

pub(super) fn execute(
    scenario: &str,
    root: &Path,
    cli: &Path,
    agent: &Path,
    image: &str,
) -> Result<()> {
    let mut guest = Guest::new(scenario, root, cli, agent, image)?;
    let outcome = if scenario == "postgresql-ownership" {
        crate::postgres_run::execute(&guest)
    } else {
        crate::become_user::execute(&guest)
    };
    let cleanup = guest.docker.close();
    match (outcome, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(anyhow::anyhow!(
            "fixture failed: {error:#}; cleanup failed: {cleanup:#}"
        )),
    }
}

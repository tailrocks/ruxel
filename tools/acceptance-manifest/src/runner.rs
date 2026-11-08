use crate::{checks, process, state};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

pub(super) struct Env {
    pub(super) root: tempfile::TempDir,
    pub(super) cli: PathBuf,
    pub(super) agent: PathBuf,
    pub(super) module: PathBuf,
    pub(super) payload: PathBuf,
}
impl Env {
    pub(super) fn new(name: &str, repo: &Path, cli: &Path, agent: &Path) -> Result<Self> {
        let data = fs::read(repo.join(format!("testdata/workloads/pr66/{name}.pkl")))?;
        ensure!(
            !crate::pins::pin(name).is_empty()
                && crate::pins::digest(&data) == crate::pins::pin(name),
            "fixture pin mismatch"
        );
        let mut builder = tempfile::Builder::new();
        builder.prefix("ruxel-pr66-manifest-");
        ensure!(
            !crate::contract::host_fixture(name),
            "privileged fixtures require host Docker orchestration"
        );
        let root = builder.tempdir()?;
        fs::create_dir_all(root.path().join("testdata/workloads/pr66"))?;
        fs::create_dir(root.path().join("schema"))?;
        fs::copy(
            repo.join("schema/Ruxel.pkl"),
            root.path().join("schema/Ruxel.pkl"),
        )?;
        let payload = root.path().join("payload");
        fs::create_dir(&payload)?;
        let value = payload.to_str().context("non-UTF8 temp path")?;
        ensure!(
            !value
                .chars()
                .any(|c| c == '"' || c == '\\' || c.is_control()),
            "unsafe temporary fixture path"
        );
        let module = root
            .path()
            .join(format!("testdata/workloads/pr66/{name}.pkl"));
        fs::write(
            &module,
            std::str::from_utf8(&data)?.replace("__RUN_ROOT__", value),
        )?;
        Ok(Self {
            root,
            cli: cli.to_path_buf(),
            agent: agent.to_path_buf(),
            module,
            payload,
        })
    }
    pub(super) fn run(&self, name: &str, mode: &str, expected: i32) -> Result<Vec<Value>> {
        let mut command = Command::new(&self.cli);
        command
            .arg(mode)
            .arg(&self.module)
            .args(["--local", "--agent"])
            .arg(&self.agent)
            .arg("--state-dir")
            .arg(self.root.path().join("state"))
            .stdin(Stdio::null());
        let (status, stdout, stderr) = process::run(&mut command, Duration::from_secs(30))?;
        ensure!(
            status.code() == Some(expected),
            "{name} {mode}: exit {status}: {}",
            String::from_utf8_lossy(&stderr)
        );
        let events: Vec<Value> = std::str::from_utf8(&stdout)?
            .lines()
            .filter(|l| !l.is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        if name == "controller-delegation" {
            crate::delegation::verify(&events, expected != 0)?;
        } else {
            checks::hosts(&events, &checks::expected_hosts(name), "recap")?;
        }
        ensure!(
            events
                .iter()
                .filter_map(|e| e.get("recap"))
                .all(|r| r["failed"] == if expected == 0 { 0 } else { 1 }),
            "failed recap mismatch"
        );
        Ok(events)
    }
}
pub fn execute(name: &str, repo: &Path, cli: &Path, agent: &Path) -> Result<()> {
    if name == "restart-actions" || name == "control-flow" {
        anyhow::ensure!(
            rustix::process::geteuid().as_raw() == 0,
            "{name} requires an isolated root fixture"
        );
    }
    if name == "control-flow" {
        let observer = Env::new(name, repo, cli, agent)?;
        crate::control_observer::execute(&observer)?;
        let cli_env = Env::new(name, repo, cli, agent)?;
        return crate::control_run::execute(&cli_env);
    }
    let env = Env::new(name, repo, cli, agent)?;
    if name == "restart-actions" {
        return crate::restart_run::execute(&env);
    }
    if name == "multihost" || name == "controller-delegation" {
        crate::plan::verify(&env, name)?;
    }

    for phase in ["fresh", "converged"] {
        let events = env.run(name, "apply", 0)?;
        checks::assert_apply(name, phase, &events, &env.payload)?;
    }
    if name == "check-semantics" {
        let path = env.payload.join("check-notify");
        ensure!(
            path.is_file() && !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "unsafe drift target"
        );
        fs::write(path, b"drift")?;
    }
    for mode in ["check", "diff"] {
        let before = state::snapshot(&env.payload)?;
        let fixture_before = state::fixture_snapshot(env.root.path())?;

        let status = if name == "controller-delegation" {
            8
        } else {
            0
        };
        let events = env.run(name, mode, status)?;
        ensure!(
            state::snapshot(&env.payload)? == before,
            "{mode} mutated payload"
        );
        ensure!(
            state::fixture_snapshot(env.root.path())? == fixture_before,
            "{mode} mutated fixture or schema"
        );

        checks::assert_preview(name, mode, &events)?;
    }
    if name == "check-semantics" {
        let events = env.run(name, "apply", 0)?;
        checks::assert_apply(name, "drift_repair", &events, &env.payload)?;
    }
    Ok(())
}

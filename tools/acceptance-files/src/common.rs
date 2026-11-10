use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

pub struct Env {
    pub root: tempfile::TempDir,
    pub cli: PathBuf,
    pub agent: PathBuf,
    pub module: PathBuf,
    pub payload: PathBuf,
}

impl Env {
    pub fn new(cli: &Path, agent: &Path, subdir: &str) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("ruxel-pr66-files-")
            .tempdir()?;
        let dir = root.path().join(subdir);
        fs::create_dir_all(&dir)?;
        let payload = root.path().join("payload");
        fs::create_dir(&payload)?;
        Ok(Self {
            module: dir.join("workload.pkl"),
            root,
            cli: cli.to_path_buf(),
            agent: agent.to_path_buf(),
            payload,
        })
    }

    pub fn run(&self, mode: &str) -> Result<(Vec<Value>, Value)> {
        let mut command = Command::new(&self.cli);
        command
            .arg(mode)
            .arg(&self.module)
            .args(["--local", "--agent"])
            .arg(&self.agent)
            .arg("--state-dir")
            .arg(self.root.path().join("state"))
            .stdin(Stdio::null());
        let (status, stdout, stderr) = crate::process::run(&mut command, Duration::from_secs(25))?;
        ensure!(
            status.success(),
            "{mode}: {}: {}\n{}",
            status,
            String::from_utf8_lossy(&stderr),
            String::from_utf8_lossy(&stdout)
        );
        let text = std::str::from_utf8(&stdout)?;
        let events: Vec<Value> = text
            .lines()
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let recaps: Vec<_> = events
            .iter()
            .filter(|event| event["host"] == "fixture")
            .filter_map(|event| event.get("recap"))
            .collect();
        ensure!(
            recaps.len() == 1 && recaps[0]["failed"] == 0,
            "{mode}: missing or failed recap"
        );
        let recap = recaps[0].clone();
        Ok((events, recap))
    }
}

pub fn quoted(path: &Path) -> Result<String> {
    Ok(serde_json::to_string(
        path.to_str().context("non-UTF8 path")?,
    )?)
}

pub fn task<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let results: Vec<_> = events
        .iter()
        .filter(|event| event["task"] == name)
        .filter_map(|event| event.get("result"))
        .collect();
    ensure!(results.len() == 1, "missing or duplicate result for {name}");
    Ok(results[0])
}

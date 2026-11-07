use crate::process;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tempfile::TempDir;
pub struct Harness<'a> {
    pub temp: TempDir,
    pub module: PathBuf,
    pub cli: &'a Path,
    pub agent: &'a Path,
}
impl<'a> Harness<'a> {
    pub fn new(root: &Path, cli: &'a Path, agent: &'a Path) -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("ruxel-isolation-")
            .tempdir()?;
        for dir in ["schema", "fixtures"] {
            fs::create_dir(temp.path().join(dir))?;
        }
        fs::copy(
            root.join("schema/Ruxel.pkl"),
            temp.path().join("schema/Ruxel.pkl"),
        )?;
        let module = temp.path().join("fixtures/scenario.pkl");
        Ok(Self {
            temp,
            module,
            cli,
            agent,
        })
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    pub fn write(&self, text: &str) -> Result<()> {
        fs::write(&self.module, text)?;
        Ok(())
    }
    pub fn run(
        &self,
        agent: &Path,
        state: &str,
        jobs: &str,
        log: Option<&Path>,
    ) -> Result<process::Output> {
        let mut c = Command::new(self.cli);
        c.arg("apply")
            .arg(&self.module)
            .arg("--local")
            .arg("--agent")
            .arg(agent)
            .arg("--state-dir")
            .arg(self.path(state))
            .arg("--jobs")
            .arg(jobs);
        if let Some(log) = log {
            c.arg("--log-file").arg(log);
        }
        process::run(&mut c, Duration::from_secs(25))
    }
    pub fn wrapper(&self, name: &str, mode: &str, log: &Path, index: usize) -> Result<PathBuf> {
        let p = self.path(name);
        fs::write(
            &p,
            format!(
                "#!/bin/sh\nexec {} --fixture {} {} {} {}\n",
                shell(&std::env::current_exe()?)?,
                mode,
                shell(self.agent)?,
                shell(log)?,
                index
            ),
        )?;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700))?;
        Ok(p)
    }
}
pub fn events(output: &process::Output, code: i32) -> Result<Vec<Value>> {
    ensure!(
        output.code == Some(code),
        "expected exit{code},got{:?}:{}\n{}",
        output.code,
        output.stderr,
        output.stdout
    );
    output
        .stdout
        .lines()
        .filter(|s| !s.trim().is_empty())
        .map(|s| serde_json::from_str(s).context("invalid JSONL event"))
        .collect()
}
pub fn pkl(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
pub fn path(path: &Path) -> Result<String> {
    pkl(path.to_str().context("non-UTF8 path")?)
}
pub fn shell(path: &Path) -> Result<String> {
    Ok(format!(
        "'{}'",
        path.to_str()
            .context("non-UTF8 path")?
            .replace('\'', "'\\''")
    ))
}
pub fn task<'a>(events: &'a [Value], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e["task"] == name && e.get("result").is_some())
        .collect()
}
pub fn recaps(events: &[Value]) -> Result<(&Value, &Value)> {
    let rows: Vec<_> = events.iter().filter(|e| e.get("recap").is_some()).collect();
    ensure!(rows.len() == 2, "missing/duplicate recaps");
    let a = rows
        .iter()
        .find(|e| e["host"] == "fixture-a")
        .context("missing a recap")?;
    let b = rows
        .iter()
        .find(|e| e["host"] == "fixture-b")
        .context("missing b recap")?;
    Ok((&a["recap"], &b["recap"]))
}
pub const TWO_PLAYS: &str = include_str!("../fixtures/two_plays.pkl");

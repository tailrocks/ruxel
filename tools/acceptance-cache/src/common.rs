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
}
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}
impl Env {
    pub fn new(cli: &Path, agent: &Path, schema: &Path) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("ruxel-pr66-cache-")
            .tempdir()?;
        fs::copy(schema, root.path().join("Ruxel.pkl"))?;
        Ok(Self {
            root,
            cli: cli.to_path_buf(),
            agent: agent.to_path_buf(),
        })
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    pub fn write(&self, name: &str, source: &str, args: &[(&str, String)]) -> Result<PathBuf> {
        let mut text = source.to_owned();
        for (key, value) in args {
            text = text.replace(key, value);
        }
        let file = self.path(name);
        fs::write(&file, text)?;
        Ok(file)
    }
    pub fn run(&self, module: &Path, state: &str, options: &[&str]) -> Result<Output> {
        let mut command = Command::new(&self.cli);
        command
            .arg("apply")
            .arg(module)
            .args(["--local", "--agent"])
            .arg(&self.agent)
            .arg("--state-dir")
            .arg(self.path(state))
            .args(options)
            .stdin(Stdio::null());
        let (status, stdout, stderr) = crate::process::run(&mut command, Duration::from_secs(30))?;
        Ok(Output {
            status: status.code().context("CLI terminated by signal")?,
            stdout: String::from_utf8(stdout)?,
            stderr: String::from_utf8(stderr)?,
        })
    }
}
pub fn quote(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
pub fn qpath(path: &Path) -> Result<String> {
    quote(path.to_str().context("non-UTF8 fixture path")?)
}
pub fn shell_quote(path: &Path) -> Result<String> {
    Ok(format!(
        "'{}'",
        path.to_str()
            .context("non-UTF8 path")?
            .replace('\'', "'\\''")
    ))
}
pub fn events(text: &str) -> Result<Vec<Value>> {
    Ok(text
        .lines()
        .filter(|v| !v.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?)
}
pub fn task<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let results: Vec<_> = events
        .iter()
        .filter(|event| event["task"] == name)
        .collect();
    ensure!(results.len() == 1, "missing or duplicate task {name}");
    results[0].get("result").context("task result missing")
}
pub fn scan(root: &Path, secrets: &[&str], no_ledger: bool) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for item in fs::read_dir(root)? {
        let path = item?.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "unexpected symlink in state"
        );
        if metadata.is_dir() {
            scan(&path, secrets, no_ledger)?;
        } else if metadata.is_file() {
            ensure!(
                !no_ledger || path.file_name().is_none_or(|n| n != "ledger.json"),
                "private operation cached"
            );
            let data = fs::read(&path)?;
            for secret in secrets {
                ensure!(
                    !data
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "synthetic secret persisted in state"
                );
            }
        } else {
            anyhow::bail!("unexpected state entry")
        }
    }
    Ok(())
}
pub fn no_leak(text: &str, secrets: &[&str]) -> Result<()> {
    ensure!(
        secrets.iter().all(|value| !text.contains(value)),
        "synthetic secret leaked in output"
    );
    Ok(())
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;

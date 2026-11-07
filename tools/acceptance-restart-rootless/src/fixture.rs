use crate::{observations, process, state};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
const PIN: &str = "8188b88f5da95d7326e6b07efdd00908c09ee11958ff1e52a695e3c5a4d14d5b";
const OWNERS: &str = "owner = \"root\"; group = \"root\"; ";

fn variant(source: &[u8], contract: &Value, payload: &str) -> Result<String> {
    let digest: String = Sha256::digest(source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let expected: Value = serde_json::from_str(include_str!(
        "../../acceptance-manifest/restart-contract.json"
    ))?;
    ensure!(
        digest == PIN && *contract == expected,
        "restart fixture/contract changed"
    );
    ensure!(
        !payload
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control()),
        "unsafe payload path"
    );
    let text = std::str::from_utf8(source)?;
    ensure!(
        text.matches(OWNERS).count() == 1,
        "root ownership substitution mismatch"
    );
    Ok(text
        .replacen(OWNERS, "", 1)
        .replace("__RUN_ROOT__", payload))
}
pub struct Fixture {
    root: tempfile::TempDir,
    cli: PathBuf,
    agent: PathBuf,
    module: PathBuf,
    payload: PathBuf,
}
impl Fixture {
    pub fn new(repo: &Path, cli: PathBuf, agent: PathBuf) -> Result<Self> {
        ensure!(
            rustix::process::getuid().as_raw() != 0,
            "rootless proof requires an unprivileged user"
        );
        let root = tempfile::Builder::new()
            .prefix("ruxel-restart-rootless-")
            .tempdir()?;
        let payload = root.path().join("payload");
        fs::create_dir(&payload)?;
        fs::create_dir(root.path().join("schema"))?;
        fs::copy(
            repo.join("schema/Ruxel.pkl"),
            root.path().join("schema/Ruxel.pkl"),
        )?;
        fs::create_dir_all(root.path().join("testdata/workloads/pr66"))?;
        let source = fs::read(repo.join("testdata/workloads/pr66/restart-actions.pkl"))?;
        let manifest: Value = serde_json::from_slice(&fs::read(
            repo.join("testdata/workloads/pr66/expectations.json"),
        )?)?;
        let text = variant(
            &source,
            &manifest["scenarios"]["restart-actions"],
            payload.to_str().context("nonUTF8 payload")?,
        )?;
        let module = root
            .path()
            .join("testdata/workloads/pr66/restart-actions-rootless.pkl");
        fs::write(&module, text)?;
        Ok(Self {
            root,
            cli,
            agent,
            module,
            payload,
        })
    }
    fn run(&self, mode: &str) -> Result<Vec<Value>> {
        let mut command = Command::new(&self.cli);
        command
            .arg(mode)
            .arg(&self.module)
            .args(["--local", "--synthetic-secrets", "--agent"])
            .arg(&self.agent)
            .arg("--state-dir")
            .arg(self.root.path().join("state"))
            .stdin(Stdio::null());
        let (status, out, err) = process::run(&mut command, Duration::from_secs(30))?;
        ensure!(
            status.code() == Some(0),
            "{mode} failed: {status}: {}",
            String::from_utf8_lossy(&err)
        );
        ensure!(out.ends_with(b"\n"), "truncated event output");
        let events: Vec<Value> = std::str::from_utf8(&out)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        observations::recaps(&events)?;
        Ok(events)
    }
    pub fn replay(&self) -> Result<()> {
        for changed in [true, false] {
            observations::apply(&self.run("apply")?, changed)?;
            observations::file(&self.payload.join("restart"))?;
        }
        let path = self.payload.join("restart");
        observations::file(&path)?;
        fs::write(&path, b"drift")?;
        for mode in ["check", "diff"] {
            let before = state::snapshot(&self.payload)?;
            let fixture = state::fixture_snapshot(self.root.path())?;
            self.run(mode)?;
            ensure!(
                before == state::snapshot(&self.payload)?
                    && fixture == state::fixture_snapshot(self.root.path())?,
                "{mode} mutated fixture"
            );
        }
        observations::apply(&self.run("apply")?, true)?;
        observations::file(&path)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_pins_and_only_rootless_ownership_substitution() -> Result<()> {
        let source = include_bytes!("../../../testdata/workloads/pr66/restart-actions.pkl");
        let contract = serde_json::from_str(include_str!(
            "../../acceptance-manifest/restart-contract.json"
        ))?;
        let text = variant(source, &contract, "/synthetic")?;
        ensure!(!text.contains(OWNERS) && text.contains("mode = \"0755\""));
        ensure!(
            text == std::str::from_utf8(source)?
                .replacen(OWNERS, "", 1)
                .replace("__RUN_ROOT__", "/synthetic")
        );
        ensure!(variant(b"changed", &contract, "/synthetic").is_err());
        ensure!(variant(source, &serde_json::json!({}), "/synthetic").is_err());
        ensure!(variant(source, &contract, "/quote\"").is_err());
        Ok(())
    }
}

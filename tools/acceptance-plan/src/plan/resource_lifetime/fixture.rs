use crate::process;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const MARKER: &str = "synthetic-lifetime-fixture-491837";
const ENCODED: &str = "c3ludGhldGljLWxpZmV0aW1lLWZpeHR1cmUtNDkxODM3";
const DOUBLE: &str = "YzNsdWRHaGxkR2xqTFd4cFptVjBhVzFsTFdacGVIUjFjbVV0TkRreE9ETTM=";
const SECRETS: &[&str] = &[MARKER, ENCODED, DOUBLE];

pub(super) struct Fixture<'a> {
    temp: tempfile::TempDir,
    cli: &'a Path,
    agent: &'a Path,
    sequence: usize,
}
impl<'a> Fixture<'a> {
    pub(super) fn new(root: &Path, cli: &'a Path, agent: &'a Path) -> Result<Self> {
        let temp = tempfile::tempdir()?;
        fs::copy(root.join("schema/Ruxel.pkl"), temp.path().join("Ruxel.pkl"))?;
        Ok(Self {
            temp,
            cli,
            agent,
            sequence: 0,
        })
    }
    pub(super) fn base(&self) -> &Path {
        self.temp.path()
    }
    pub(super) fn state(&self, name: &str) -> PathBuf {
        self.base().join(name)
    }
    pub(super) fn run(&mut self, plays: &str, state: &str, code: i32) -> Result<Vec<Value>> {
        self.sequence += 1;
        let module = self.base().join(format!("case-{}.pkl", self.sequence));
        let log = self.base().join(format!("case-{}.jsonl", self.sequence));
        fs::write(
            &module,
            format!(
                r#"import "Ruxel.pkl"
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["a"] {{ address = "127.0.0.1" }}; ["b"] {{ address = "127.0.0.1" }} }} }}
 plays {{ {plays} }}
}} }}"#
            ),
        )?;
        let events = process::run_checked(
            self.cli,
            self.agent,
            &module,
            &self.state(state),
            "apply",
            Duration::from_secs(30),
            &[OsStr::new("--log-file"), log.as_os_str()],
            SECRETS,
            code,
        )?;
        let log = fs::read_to_string(log)?;
        ensure!(
            SECRETS.iter().all(|secret| !log.contains(secret)),
            "sensitive form leaked to log"
        );
        Ok(events)
    }
}

pub(super) fn text(path: &Path) -> Result<String> {
    Ok(serde_json::to_string(
        path.to_str().context("path is not UTF-8")?,
    )?)
}
pub(super) fn literal(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
pub(super) fn play(host: &str, tasks: &str) -> String {
    format!(r#"new {{ name = "fixture"; targets {{ "{host}" }}; tasks {{ {tasks} }} }}"#)
}
pub(super) fn write(path: &Path, content: &str, delegate: &str) -> Result<String> {
    Ok(format!(
        r#"new {{ name = "write"; noLog = true; {delegate} operation = new Ruxel.Copy {{ dest = {}; content = {} }} }}"#,
        text(path)?,
        literal(content)?
    ))
}
pub(super) fn read(path: &Path) -> Result<String> {
    Ok(format!(
        r#"new {{ name = "read"; register = "readback"; operation = new Ruxel.Slurp {{ src = {} }} }}
new {{ name = "show"; operation = new Ruxel.Debug {{ msg = new Ruxel.Call {{ `function` = "base64Encode"; args {{ new Ruxel.Ref {{ path {{ "results"; "readback"; "content" }} }} }} }} }} }}"#,
        text(path)?
    ))
}
pub(super) fn censored(events: &[Value], count: usize) -> Result<()> {
    let actual = events
        .iter()
        .filter(|event| event["result"]["censored"] == true)
        .count();
    ensure!(
        actual == count,
        "expected {count} censored task events, got {actual}"
    );
    Ok(())
}

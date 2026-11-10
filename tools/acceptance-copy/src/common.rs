use crate::process;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;
mod paths;
pub use paths::{mode, path, pkl, shell};
pub struct Harness<'a> {
    pub temp: TempDir,
    cli: &'a Path,
    agent: &'a Path,
    pub module: PathBuf,
    pub payload: PathBuf,
}
impl<'a> Harness<'a> {
    pub fn new(cli: &'a Path, agent: &'a Path) -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("ruxel-copy-acceptance-")
            .tempdir()?;
        for dir in ["fixtures", "payload"] {
            fs::create_dir(temp.path().join(dir))?;
        }
        let module = temp.path().join("fixtures/copy.pkl");
        let payload = temp.path().join("payload");
        Ok(Self {
            temp,
            cli,
            agent,
            module,
            payload,
        })
    }
    pub fn write(&self, tasks: &str) -> Result<()> {
        let text = format!(
            r#"import "modulepath:/ruxel/engine/Engine.pkl" as Ruxel
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["fixture"] {{ address = "127.0.0.1" }} }}; groups {{ ["all"] {{ hosts {{ "fixture" }} }} }} }}
 plays {{ new {{ name = "copy acceptance"; targets {{ "all" }}; tasks {{ {tasks} }} }} }}
}} }}
"#
        );
        fs::write(&self.module, text)?;
        Ok(())
    }
    pub fn events(&self, mode: &str, limit: u64) -> Result<Vec<Value>> {
        let events = process::run(
            self.cli,
            self.agent,
            &self.module,
            &self.temp.path().join("state"),
            mode,
            Duration::from_secs(limit),
        )?;
        validate_recap(&events)?;
        Ok(events)
    }
    pub fn run(&self, mode: &str) -> Result<Value> {
        let events = self.events(mode, 20)?;
        Ok(task_result(&events, "copy")?.clone())
    }
    pub fn private_run(&self, mode: &str) -> Result<Value> {
        let events = process::run(
            self.cli,
            self.agent,
            &self.module,
            &self.temp.path().join("state"),
            mode,
            Duration::from_secs(20),
        )?;
        validate_recap_for_host(&events, "[censored]")?;
        ensure!(
            events.iter().all(|event| event["host"] == "[censored]"),
            "private copy exposed host metadata"
        );
        let result = task_result(&events, "[censored]")?;
        let changed = result["changed"].as_bool().context("missing change flag")?;
        ensure!(
            events.len() == 2
                && result == &serde_json::json!({"censored":true,"changed":changed,"failed":false}),
            "private copy result or event count mismatch"
        );
        Ok(result.clone())
    }
    pub fn snapshot(&self) -> Result<BTreeMap<OsString, (u32, Vec<u8>)>> {
        let mut result = BTreeMap::new();
        for entry in fs::read_dir(&self.payload)? {
            let entry = entry?;
            if entry.path().is_file() {
                result.insert(
                    entry.file_name(),
                    (mode(&entry.path())?, fs::read(entry.path())?),
                );
            }
        }
        Ok(result)
    }
    pub fn predict(&self, changed: bool) -> Result<()> {
        let before = self.snapshot()?;
        for mode in ["check", "diff"] {
            ensure!(
                self.run(mode)?["changed"] == changed && self.snapshot()? == before,
                "{mode}: incorrect change prediction or payload mutation"
            );
        }
        Ok(())
    }
}
fn validate_recap(events: &[Value]) -> Result<()> {
    validate_recap_for_host(events, "fixture")
}
fn validate_recap_for_host(events: &[Value], host: &str) -> Result<()> {
    let recaps: Vec<_> = events
        .iter()
        .filter(|e| e["host"] == host && e.get("recap").is_some())
        .collect();
    ensure!(
        recaps.len() == 1 && recaps[0]["recap"]["failed"] == 0,
        "missing, duplicate, or failed fixture recap"
    );
    Ok(())
}
fn task_result<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let matches: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == name && e.get("result").is_some())
        .collect();
    ensure!(
        matches.len() == 1,
        "missing or duplicate task result: {name}"
    );
    Ok(&matches[0]["result"])
}
#[cfg(test)]
mod tests;

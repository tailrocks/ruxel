#[cfg(test)]
use anyhow::Context;
use anyhow::{Result, ensure};
#[cfg(test)]
use serde_json::Value;
use std::{fs, os::unix::fs::MetadataExt, path::Path};
#[cfg(test)]
const PIN: &str = "35e426515163c412607a2c333f206f73cefac8f65b91f34170b6c232efe6fa2e";

#[cfg(test)]
fn validate_inputs(fixture: &[u8], case: &Value) -> Result<()> {
    let digest = crate::pins::digest(fixture);
    let expected: Value = serde_json::from_str(include_str!("../restart-contract.json"))?;
    ensure!(
        digest == PIN && *case == expected,
        "restart fixture or contract changed"
    );
    Ok(())
}
fn metadata(regular: bool, uid: u32, gid: u32, mode: u32, bytes: &[u8]) -> Result<()> {
    ensure!(
        regular && uid == 0 && gid == 0 && mode == 0o755 && bytes == b"#!/bin/sh\nexit 0\n",
        "restart file mismatch"
    );
    Ok(())
}
pub(super) fn file(path: &Path) -> Result<()> {
    let info = fs::symlink_metadata(path)?;
    ensure!(
        info.file_type().is_file(),
        "restart path not a regular file"
    );
    metadata(
        true,
        info.uid(),
        info.gid(),
        info.mode() & 0o7777,
        &fs::read(path)?,
    )
}
#[cfg(test)]
fn events(events: &[Value]) -> Result<()> {
    let recaps: Vec<_> = events
        .iter()
        .filter(|event| event.get("recap").is_some())
        .collect();
    ensure!(
        recaps.len() == 1 && recaps[0]["host"] == "fixture" && recaps[0]["recap"]["failed"] == 0,
        "restart recap mismatch"
    );
    let rows: Vec<_> = events
        .iter()
        .filter(|event| event["task"] == "Restart synthetic workload units")
        .collect();
    ensure!(
        rows.len() == 1 && rows[0]["host"] == "fixture",
        "restart task missing or duplicated"
    );
    visible(&rows[0]["result"])?;
    let results = rows[0]["result"]["results"]
        .as_array()
        .context("missing restart loop results")?;
    ensure!(results.len() == 36, "restart loop cardinality mismatch");
    for (index, result) in results.iter().enumerate() {
        visible(result)?;
        ensure!(
            result["item"] == format!("{:02}", index + 1) && result["rc"] == 0,
            "restart loop result mismatch"
        );
    }
    Ok(())
}
#[cfg(test)]
fn visible(result: &Value) -> Result<()> {
    ensure!(
        result["censored"] != true
            && result["redacted"] != true
            && result["skipped"] != true
            && result["failed"] != true,
        "restart result is censored, skipped, or failed"
    );
    Ok(())
}
#[cfg(test)]
fn preview(
    before: &std::collections::BTreeMap<std::path::PathBuf, crate::state::Entry>,
    after: &std::collections::BTreeMap<std::path::PathBuf, crate::state::Entry>,
) -> Result<()> {
    ensure!(before == after, "preview mutated fixture state");
    Ok(())
}
#[cfg(test)]
trait Lifecycle {
    type Snapshot: PartialEq;
    fn apply(&mut self) -> Result<()>;
    fn observe(&mut self) -> Result<()>;
    fn drift(&mut self) -> Result<()>;
    fn snapshot(&mut self) -> Result<Self::Snapshot>;
    fn preview(&mut self, mode: &str) -> Result<()>;
}
#[cfg(test)]
fn lifecycle<T: Lifecycle>(run: &mut T) -> Result<()> {
    for _ in 0..2 {
        run.apply()?;
        run.observe()?;
    }
    run.drift()?;
    for mode in ["check", "diff"] {
        let before = run.snapshot()?;
        run.preview(mode)?;
        ensure!(run.snapshot()? == before, "{mode} mutated state");
    }
    run.apply()?;
    run.observe()?;
    Ok(())
}
#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "restart_lifecycle_tests.rs"]
mod lifecycle_tests;

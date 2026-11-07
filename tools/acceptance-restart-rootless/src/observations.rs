use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, os::unix::fs::MetadataExt, path::Path};

pub fn file(path: &Path) -> Result<()> {
    let info = fs::symlink_metadata(path)?;
    ensure!(info.file_type().is_file(), "restart is not a regular file");
    ensure!(
        info.uid() == rustix::process::getuid().as_raw()
            && info.gid() == rustix::process::getgid().as_raw()
            && info.mode() & 0o7777 == 0o755
            && fs::read(path)? == b"#!/bin/sh\nexit 0\n",
        "rootless restart metadata or bytes mismatch"
    );
    Ok(())
}
fn visible(result: &Value) -> Result<()> {
    ensure!(
        result.is_object()
            && ["censored", "redacted", "skipped", "failed"]
                .iter()
                .all(|key| result[*key] != true),
        "restart result censored/skipped/failed"
    );
    Ok(())
}
pub fn recaps(events: &[Value]) -> Result<()> {
    let rows: Vec<_> = events.iter().filter(|e| e.get("recap").is_some()).collect();
    ensure!(
        rows.len() == 1 && rows[0]["host"] == "fixture" && rows[0]["recap"]["failed"] == 0,
        "restart recap mismatch"
    );
    Ok(())
}
pub fn apply(events: &[Value], copy_changed: bool) -> Result<()> {
    recaps(events)?;
    let rows: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "Restart synthetic workload units")
        .collect();
    ensure!(
        rows.len() == 1 && rows[0]["host"] == "fixture",
        "restart loop event cardinality"
    );
    visible(&rows[0]["result"])?;
    let results = rows[0]["result"]["results"]
        .as_array()
        .context("missing restart results")?;
    ensure!(results.len() == 36, "restart result cardinality");
    for (index, result) in results.iter().enumerate() {
        visible(result)?;
        ensure!(
            result["item"] == format!("{:02}", index + 1) && result["rc"] == 0,
            "restart order/status mismatch"
        );
    }
    let copies: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "Install synthetic restart command")
        .collect();
    ensure!(
        copies.len() == 1 && copies[0]["host"] == "fixture",
        "copy event cardinality"
    );
    visible(&copies[0]["result"])?;
    ensure!(
        copies[0]["result"]["changed"] == copy_changed,
        "copy convergence mismatch"
    );
    Ok(())
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;

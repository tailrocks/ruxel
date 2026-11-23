pub use crate::checks_preview::assert_preview;

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

pub fn hosts(events: &[Value], expected: &BTreeSet<String>, key: &str) -> Result<()> {
    let entries: Vec<_> = events.iter().filter(|e| e.get(key).is_some()).collect();
    let observed: BTreeSet<_> = entries
        .iter()
        .filter_map(|e| e["host"].as_str().map(str::to_owned))
        .collect();
    ensure!(
        entries.len() == expected.len() && observed == *expected,
        "missing or duplicate {key} hosts"
    );
    Ok(())
}
pub fn task<'a>(events: &'a [Value], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["task"] == name && event["host"] == "fixture")
        .filter_map(|event| event.get("result"))
        .collect()
}
pub fn assert_apply(name: &str, phase: &str, events: &[Value], payload: &Path) -> Result<()> {
    match name {
        "check-semantics" => {
            let file = payload.join("check-notify");
            ensure!(
                std::fs::symlink_metadata(&file)?.file_type().is_file()
                    && std::fs::read(file)? == b"synthetic",
                "file observation mismatch"
            );
            let recap = events
                .iter()
                .find_map(|e| e.get("recap"))
                .context("missing recap")?;
            let changed = recap["changed"].as_u64().context("missing change count")? > 0;
            ensure!(
                changed == (phase != "converged"),
                "change expectation mismatch"
            );
        }
        "controller-delegation" => {
            crate::delegation::verify(events, false)?;
        }
        "multihost" => {
            let rows: Vec<_> = events
                .iter()
                .filter(|e| {
                    e["task"] == "Run one-second independent host work" && e.get("result").is_some()
                })
                .collect();
            let expected = expected_hosts(name);
            let observed: BTreeSet<_> = rows
                .iter()
                .filter_map(|e| e["host"].as_str().map(str::to_owned))
                .collect();
            ensure!(
                rows.len() == expected.len()
                    && observed == expected
                    && rows.iter().all(|e| e["result"]["rc"] == 0),
                "host task mismatch"
            );
        }
        _ => anyhow::bail!("unsupported case"),
    }
    Ok(())
}
pub fn expected_hosts(name: &str) -> BTreeSet<String> {
    if name == "multihost" {
        (1..=6).map(|n| format!("fixture-{n}")).collect()
    } else {
        BTreeSet::from(["fixture".into()])
    }
}
#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;

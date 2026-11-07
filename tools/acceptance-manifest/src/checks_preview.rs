use crate::checks::{expected_hosts, task};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;

pub fn assert_preview(name: &str, mode: &str, events: &[Value]) -> Result<()> {
    match name {
        "check-semantics" => {
            let handler = task(events, "Record predicted handler");
            ensure!(
                handler.len() == 1
                    && handler[0]["msg"] == "predicted handler ran"
                    && handler[0]["failed"] != true
                    && handler[0]["skipped"] != true,
                "predicted handler missing"
            );
            ensure!(
                events
                    .iter()
                    .any(|e| e["recap"]["changed"].as_u64().is_some_and(|v| v > 0)),
                "change not predicted"
            );
            if mode == "diff" {
                ensure!(
                    events
                        .iter()
                        .any(|e| e["result"]["diff"].as_str().is_some_and(|v| !v.is_empty())),
                    "diff missing"
                );
            }
        }
        "controller-delegation" => {
            crate::delegation::verify(events, true)?;
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
                    && rows
                        .iter()
                        .all(|e| e["result"]["skipped"] == true && e["result"]["changed"] == false),
                "preview host results mismatch"
            );
        }
        _ => anyhow::bail!("unsupported case"),
    }
    Ok(())
}

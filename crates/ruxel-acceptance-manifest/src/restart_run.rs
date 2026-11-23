use crate::{restart, runner::Env, state};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::fs;

pub(super) fn execute(env: &Env) -> Result<()> {
    let path = env.payload.join("restart");
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if matches!(phase, "drift_repair" | "check") {
            ensure!(
                fs::symlink_metadata(&path)?.file_type().is_file(),
                "unsafe drift target"
            );
            fs::write(&path, b"drift")?;
        }
        let preview = matches!(phase, "check" | "diff");
        let before = state::snapshot(&env.payload)?;
        let fixture = state::fixture_snapshot(env.root.path())?;
        let mode = if preview { phase } else { "apply" };
        let rows = env.run("restart-actions", mode, 0)?;
        events(&rows, phase, &path)?;
        if preview {
            ensure!(
                state::snapshot(&env.payload)? == before,
                "{phase} mutated payload"
            );
            ensure!(
                state::fixture_snapshot(env.root.path())? == fixture,
                "{phase} mutated fixture"
            );
        } else {
            restart::file(&path)?;
        }
        println!("PASS restart-actions {phase}");
    }
    Ok(())
}

fn events(events: &[Value], phase: &str, path: &std::path::Path) -> Result<()> {
    ensure!(events.len() == 3, "restart event count mismatch");
    let task = |name: &str| -> Result<&Value> {
        let rows: Vec<_> = events
            .iter()
            .filter(|v| v["task"] == name && v["host"] == "fixture")
            .collect();
        ensure!(rows.len() == 1, "missing or duplicate {name}");
        Ok(&rows[0]["result"])
    };
    let preview = matches!(phase, "check" | "diff");
    let copy = task("Install synthetic restart command")?;
    ensure!(
        copy["changed"] == (phase != "converged") && copy["failed"] != true,
        "restart copy mismatch"
    );
    let result = task("Restart synthetic workload units")?;
    ensure!(
        result["changed"] == !preview && result["skipped"] == preview && result["failed"] == false,
        "restart task mismatch"
    );
    let rows = result["results"]
        .as_array()
        .context("missing restart results")?;
    ensure!(rows.len() == 36, "restart result count mismatch");
    let path = path.to_str().context("non-UTF8 restart path")?;
    for (index, row) in rows.iter().enumerate() {
        let item = format!("{:02}", index + 1);
        ensure!(
            row["item"] == item
                && row["cmd"] == format!("{path} {item}")
                && row["rc"] == 0
                && row["changed"] == !preview
                && row["skipped"].as_bool().unwrap_or(false) == preview,
            "restart result mismatch"
        );
        ensure!(
            ["failed", "censored", "redacted", "cached"]
                .iter()
                .all(|key| row[*key] != true),
            "invalid restart result"
        );
    }
    let recaps: Vec<_> = events.iter().filter(|v| v.get("recap").is_some()).collect();
    ensure!(
        recaps.len() == 1
            && recaps[0]["host"] == "fixture"
            && recaps[0]["recap"]["failed"] == 0
            && recaps[0]["recap"]["changed"]
                == if phase == "converged" || preview {
                    1
                } else {
                    2
                },
        "restart recap mismatch"
    );
    Ok(())
}

#[cfg(test)]
#[path = "restart_run_tests.rs"]
mod tests;

use crate::common::{Harness, TWO_PLAYS, events};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};
pub fn check(cli: &Path, agent: &Path, post: bool) -> Result<()> {
    let h = Harness::new(cli, agent)?;
    h.write(TWO_PLAYS)?;
    for failed in 0..2 {
        let log = h.path(&format!("launch-{failed}"));
        let mode = if post { "post-handshake" } else { "handshake" };
        let wrapper = h.wrapper(&format!("wrapper-{failed}"), mode, &log, failed)?;
        let output = h.run(&wrapper, &format!("state-{failed}"), "1", None)?;
        let events = events(&output, 7)?;
        validate(&events, failed)?;
        let rows: Vec<Value> = fs::read_to_string(&log)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let indices: BTreeSet<_> = rows.iter().map(|r| r[0].as_u64()).collect();
        let states: BTreeSet<_> = rows.iter().map(|r| r[1].as_str()).collect();
        ensure!(
            indices == BTreeSet::from([Some(0), Some(1)])
                && states.len() == 2
                && rows.iter().filter(|r| r[0] == failed).count() == 1,
            "failed host retried or missing host launch"
        );
    }
    println!(
        "PASS {} isolation: both orderings, healthy host completes both plays, failed host not retried",
        if post { "post-handshake" } else { "handshake" }
    );
    Ok(())
}
fn validate(events: &[Value], failed: usize) -> Result<()> {
    let names = ["fixture-a", "fixture-b"];
    let healthy = names[1 - failed];
    let tasks: Vec<_> = events
        .iter()
        .filter(|event| event.get("result").is_some())
        .collect();
    ensure!(
        tasks.len() == 4,
        "healthy host did not complete both commands and assertions"
    );
    for (row, (name, stdout)) in tasks.iter().zip([
        ("first command", Some("first")),
        ("assert first", None),
        ("second command", Some("second")),
        ("assert second", None),
    ]) {
        ensure!(row["result"]["failed"] == false, "healthy task failed");
        if row["task"] == "[censored]" {
            ensure!(
                row["host"] == "[censored]" && row["result"]["censored"] == true,
                "private isolation metadata or result was visible"
            );
        } else {
            ensure!(
                row["task"] == name && row["host"] == healthy,
                "healthy task identity mismatch"
            );
            if let Some(stdout) = stdout {
                ensure!(
                    row["result"]["stdout"] == stdout,
                    "healthy command output mismatch"
                );
            }
        }
    }
    let recaps: Vec<_> = events
        .iter()
        .filter_map(|event| event.get("recap"))
        .collect();
    ensure!(recaps.len() == 2, "missing or duplicate isolation recaps");
    let healthy: Vec<_> = recaps.iter().filter(|recap| recap["failed"] == 0).collect();
    let failed: Vec<_> = recaps
        .iter()
        .filter(|recap| recap["failed"].as_u64().is_some_and(|n| n > 0))
        .collect();
    ensure!(
        healthy.len() == 1
            && failed.len() == 1
            && healthy[0]["ok"] == 4
            && failed[0]["ok"] == 0
            && failed[0]["unreachable"] == 1,
        "incorrect failed/healthy recap"
    );
    Ok(())
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;

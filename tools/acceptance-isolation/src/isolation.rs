use crate::common::{Harness, TWO_PLAYS, events, recaps, task};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};
pub fn check(root: &Path, cli: &Path, agent: &Path, post: bool) -> Result<()> {
    let h = Harness::new(root, cli, agent)?;
    h.write(TWO_PLAYS)?;
    for failed in 0..2 {
        let log = h.path(&format!("launch-{failed}"));
        let mode = if post { "post-handshake" } else { "handshake" };
        let wrapper = h.wrapper(&format!("wrapper-{failed}"), mode, &log, failed)?;
        let output = h.run(&wrapper, &format!("state-{failed}"), "1", None)?;
        let events = events(&output, 8)?;
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
    for (name, text) in [("first command", "first"), ("second command", "second")] {
        let rows = task(events, name);
        ensure!(
            rows.len() == 1 && rows[0]["host"] == healthy && rows[0]["result"]["stdout"] == text,
            "healthy host missing {name}"
        );
    }
    let (a, b) = recaps(events)?;
    let rows = [a, b];
    ensure!(
        rows[failed]["failed"].as_u64().is_some_and(|n| n > 0) && rows[1 - failed]["failed"] == 0,
        "incorrect failed/healthy recap"
    );
    Ok(())
}

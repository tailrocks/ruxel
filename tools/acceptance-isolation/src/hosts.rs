use crate::common::{Harness, events, path, pkl, recaps, task};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};
pub fn check(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let h = Harness::new(root, cli, agent)?;
    let names: Vec<_> = (1..=4).map(|i| format!("fixture-{i}")).collect();
    let mut hosts = Vec::new();
    for name in &names {
        let n = pkl(name)?;
        hosts.push(format!("[{n}] {{ address = \"127.0.0.1\"; vars {{ [\"token\"] = new Ruxel.Literal {{ value = {n} }} }} }}"));
    }
    let log = h.path("intervals");
    let args = [
        path(&std::env::current_exe()?)?,
        pkl("--fixture")?,
        pkl("instrument")?,
        path(&log)?,
    ]
    .join("; ");
    let source = include_str!("../fixtures/hosts.pkl")
        .replace("HOSTS", &hosts.join("\n"))
        .replace(
            "HOSTNAMES",
            &names
                .iter()
                .map(|s| pkl(s))
                .collect::<Result<Vec<_>>>()?
                .join("; "),
        )
        .replace("ARGV", &args);
    h.write(&source)?;
    let e = events(&h.run(agent, "state", "2", None)?, 0)?;
    let facts = task(&e, "read distinct fact");
    ensure!(
        facts.len() == 4
            && facts
                .iter()
                .map(|e| e["host"].as_str())
                .collect::<BTreeSet<_>>()
                == names.iter().map(|n| Some(n.as_str())).collect(),
        "missing/duplicate facts"
    );
    ensure!(
        facts.iter().all(|e| e["result"]["msg"] == e["host"]),
        "host facts crossed boundaries"
    );
    let first: Vec<_> = e
        .iter()
        .enumerate()
        .filter(|(_, e)| e["task"] == "read distinct fact")
        .map(|(i, _)| i)
        .collect();
    let second: Vec<_> = e
        .iter()
        .enumerate()
        .filter(|(_, e)| e["task"] == "second play")
        .collect();
    ensure!(
        second.len() == 4
            && second
                .iter()
                .map(|(_, e)| e["host"].as_str())
                .collect::<BTreeSet<_>>()
                == names.iter().map(|n| Some(n.as_str())).collect()
            && first.iter().max().context("no first play")? < &second[0].0,
        "play barrier violated"
    );
    intervals(&fs::read_to_string(log)?)?;
    shutdown(&h)?;
    println!(
        "PASS host isolation: distinct facts, bounded overlap, play barrier, drained shutdown errors"
    );
    Ok(())
}
fn intervals(text: &str) -> Result<()> {
    let mut rows: Vec<Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(rows.len() == 8, "missing instrumentation events");
    rows.sort_by_key(|r| r[2].as_u64());
    let mut active = BTreeSet::new();
    let mut peak = 0;
    for row in rows {
        let pid = row[1].as_u64().context("missing process id")?;
        if row[0] == "start" {
            ensure!(active.insert(pid), "duplicate start");
            peak = peak.max(active.len());
        } else {
            ensure!(
                row[0] == "end" && active.remove(&pid),
                "unmatched process end"
            );
        }
    }
    ensure!(
        active.is_empty() && peak == 2,
        "expected overlap bounded by2, got{peak}"
    );
    Ok(())
}
fn shutdown(h: &Harness<'_>) -> Result<()> {
    let log = h.path("closed");
    let wrapper = h.wrapper("shutdown-wrapper", "shutdown", &log, 0)?;
    h.write(include_str!("../fixtures/shutdown.pkl"))?;
    let e = events(&h.run(&wrapper, "shutdown-state", "2", None)?, 8)?;
    let errors: Vec<_> = e.iter().filter(|e| e["event"] == "unreachable").collect();
    ensure!(
        errors.len() == 2
            && errors
                .iter()
                .map(|e| e["host"].as_str())
                .collect::<BTreeSet<_>>()
                == BTreeSet::from([Some("fixture-a"), Some("fixture-b")])
            && errors.iter().all(|e| e["unreachable"] == true
                && e["changed"] == false
                && e["msg"]
                    .as_str()
                    .is_some_and(|s| s.contains("local agent exited unsuccessfully"))),
        "missing structured shutdown errors"
    );
    let (a, b) = recaps(&e)?;
    ensure!(
        [a, b]
            .iter()
            .all(|r| r["failed"] == 1 && r["ok"].as_u64().is_some_and(|n| n >= 1)),
        "shutdown lost completed recaps"
    );
    let text = fs::read_to_string(log)?;
    let rows: Vec<_> = text.lines().collect();
    ensure!(
        rows.len() == 2 && rows.iter().collect::<BTreeSet<_>>().len() == 2,
        "both distinct agents did not shut down"
    );
    Ok(())
}

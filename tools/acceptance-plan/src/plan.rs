use crate::{pins, process};
mod privacy;
mod privacy_errors;
mod privacy_hosts;
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path, time::Duration};
const SCENARIOS: [&str; 14] = [
    "become-environment",
    "check-semantics",
    "control-flow",
    "controller-delegation",
    "files-content",
    "multihost",
    "performance-snapshots",
    "postgresql-ownership",
    "restart-actions",
    "shape-variants",
    "storage-ext4",
    "storage-two-tier",
    "storage-xfs",
    "system-surface",
];
pub fn run(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let files = pins::verified(root)?;
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    for (name, bytes) in files {
        let target = base.join(name);
        fs::create_dir_all(target.parent().context("missing parent")?)?;
        fs::write(target, bytes)?;
    }
    fs::create_dir(base.join("schema"))?;
    fs::copy(root.join("schema/Ruxel.pkl"), base.join("schema/Ruxel.pkl"))?;
    let payload = base.join("payload");
    fs::create_dir(&payload)?;
    let fixtures = base.join("testdata/workloads/pr66");
    let mut failures = 0;
    for name in SCENARIOS {
        let run = || -> Result<()> {
            let module = fixtures.join(format!("{name}.pkl"));
            let source = fs::read_to_string(&module)?
                .replace("__RUN_ROOT__", &payload.to_string_lossy())
                .replace("__FIXTURE_ROOT__", &fixtures.to_string_lossy());
            fs::write(&module, source)?;
            let events = process::run(
                cli,
                agent,
                &module,
                &base.join("state"),
                "plan",
                Duration::from_secs(30),
            )?;
            validate(&events, &payload)
        };
        match run() {
            Ok(()) => println!("PASS plan {name}"),
            Err(error) => {
                failures += 1;
                eprintln!("FAIL plan {name}: {error:#}");
            }
        }
    }
    println!(
        "{}/{} plans passed; execution and provider behavior not tested",
        SCENARIOS.len() - failures,
        SCENARIOS.len()
    );
    ensure!(failures == 0, "plan matrix failed");
    privacy::run(root, cli, agent)?;
    privacy_errors::run(cli, agent)?;
    privacy_hosts::run(root, cli, agent)?;
    Ok(())
}
fn validate(events: &[serde_json::Value], payload: &Path) -> Result<()> {
    ensure!(
        events.iter().any(|e| e["planned"] == true),
        "no planned task events"
    );
    let recaps: Vec<_> = events.iter().filter_map(|e| e.get("recap")).collect();
    ensure!(
        !recaps.is_empty() && recaps.iter().all(|r| r["failed"] == 0 && r["changed"] == 0),
        "missing, failed, or changed recap"
    );
    ensure!(
        fs::read_dir(payload)?.next().is_none(),
        "plan wrote to payload"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_missing_plans_failed_recaps_and_mutation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let good = vec![
            json!({"planned":true}),
            json!({"recap":{"failed":0,"changed":0}}),
        ];
        assert!(validate(&good, dir.path()).is_ok());
        assert!(validate(&good[1..], dir.path()).is_err());
        assert!(validate(&good[..1], dir.path()).is_err());
        for field in ["failed", "changed"] {
            let mut bad = good.clone();
            bad[1]["recap"][field] = json!(1);
            assert!(validate(&bad, dir.path()).is_err());
        }
        fs::write(dir.path().join("unexpected"), "x")?;
        assert!(validate(&good, dir.path()).is_err());
        Ok(())
    }
}

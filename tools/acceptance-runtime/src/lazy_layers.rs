use crate::process;
use anyhow::{Result, ensure};
use std::{path::Path, time::Duration};

pub fn run(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    for fixture in ["play", "block"] {
        let module = root.join(format!("testdata/workloads/pr66/lazy-layers/{fixture}.pkl"));
        for mode in ["apply", "check", "diff"] {
            let events = process::run(
                cli,
                agent,
                &module,
                &temp.path().join(fixture),
                mode,
                Duration::from_secs(20),
            )?;
            let tasks: Vec<_> = events
                .iter()
                .filter(|event| event.get("task").is_some())
                .collect();
            ensure!(
                tasks.len() == 3,
                "{fixture} {mode}: expected three task events"
            );
            for (event, (task, message)) in tasks.iter().zip([
                ("before", "before"),
                ("future", "available"),
                ("after", "available"),
            ]) {
                ensure!(
                    event["task"] == task
                        && event["result"]["msg"] == message
                        && event["result"]["failed"] != true,
                    "{fixture} {mode}: unexpected task result"
                );
            }
            let recaps: Vec<_> = events
                .iter()
                .filter(|event| event.get("recap").is_some())
                .collect();
            ensure!(
                recaps.len() == 1
                    && recaps[0]["recap"]["ok"] == 3
                    && recaps[0]["recap"]["failed"] == 0,
                "{fixture} {mode}: invalid recap"
            );
        }
    }
    println!("PASS lazy play and block variables across apply/check/diff");
    Ok(())
}

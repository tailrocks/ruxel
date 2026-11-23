use crate::process;
use anyhow::{Context, Result, ensure};
mod events;
use events::host_events;
use std::{ffi::OsStr, fs, path::Path, time::Duration};
const SECRET: &str = "synthetic-sensitive-cross-play-638251-";
const HOSTS: [&str; 2] = ["fixture-a", "fixture-b"];

fn no_secret(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            no_secret(&entry.path())?;
        } else {
            ensure!(
                !fs::read(entry.path())?
                    .windows(SECRET.len())
                    .any(|w| w == SECRET.as_bytes()),
                "private fact leaked in state"
            );
        }
    }
    Ok(())
}
pub fn run(cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    for dir in ["fixtures", "payload"] {
        fs::create_dir(base.join(dir))?;
    }
    let payload = base.join("payload");
    let prefix = format!("{}/", payload.display());
    let source = include_str!("../fixtures/cross_play.pkl")
        .replace("PREFIX", &serde_json::to_string(&prefix)?)
        .replace("SECRET", &serde_json::to_string(SECRET)?);
    let module = base.join("fixtures/facts.pkl");
    fs::write(&module, source)?;
    for (index, mode) in ["apply", "apply", "check", "diff", "apply"]
        .iter()
        .enumerate()
    {
        if index == 2 {
            fs::write(payload.join("fixture-a"), "drift")?;
        }
        let before = crate::snapshot::files(&payload)?;
        let log = base.join(format!("event-{index}.jsonl"));
        let args = [
            OsStr::new("--jobs"),
            OsStr::new("2"),
            OsStr::new("--log-file"),
            log.as_os_str(),
        ];
        let events = process::run_extra(
            cli,
            agent,
            &module,
            &base.join("state"),
            mode,
            Duration::from_secs(25),
            &args,
            Some(SECRET),
            0,
        )?;
        let log_text = fs::read_to_string(log)?;
        ensure!(
            !log_text.is_empty() && !log_text.contains(SECRET),
            "{mode}: missing log or private fact leaked"
        );
        let recaps = host_events(&events, None)?;
        ensure!(
            recaps.iter().all(|e| e["recap"]["failed"] == 0),
            "{mode}: failed host"
        );
        for task in ["set private fact", "consume private fact"] {
            ensure!(
                host_events(&events, Some(task))?
                    .iter()
                    .all(|e| e["result"]["censored"] == true),
                "{mode}: result not censored"
            );
        }
        let changed: Vec<_> = recaps
            .iter()
            .map(|e| {
                e["recap"]["changed"]
                    .as_u64()
                    .context("invalid changed count")
            })
            .collect::<Result<_>>()?;
        if index == 0 {
            ensure!(changed.iter().all(|n| *n > 0), "fresh hosts not changed");
        } else if index == 1 {
            ensure!(changed.iter().all(|n| *n == 0), "converged hosts changed");
        } else {
            ensure!(
                changed.iter().any(|n| *n > 0),
                "drift not predicted or repaired"
            );
        }
        let after = crate::snapshot::files(&payload)?;
        if matches!(*mode, "check" | "diff") {
            ensure!(after == before, "{mode}: modified payload");
        } else {
            let expected = HOSTS
                .iter()
                .map(|host| ((*host).into(), format!("{SECRET}{host}").into_bytes()))
                .collect();
            ensure!(
                after == expected,
                "{mode}: incorrect per-host facts or unexpected files"
            );
        }
    }
    no_secret(&base.join("state"))?;
    println!("PASS cross-play private facts, convergence and drift");
    Ok(())
}

#[cfg(test)]
#[path = "cross_play_tests.rs"]
mod tests;

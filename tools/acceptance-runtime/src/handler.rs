mod validate;
use crate::process;
use anyhow::{Context, Result, ensure};
use std::{ffi::OsStr, fs, path::Path, time::Duration};
use validate::validate;
fn snapshot(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push((
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path())?,
            ));
        }
    }
    files.sort();
    Ok(files)
}
pub fn run(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    for dir in ["schema", "fixtures", "payload"] {
        fs::create_dir(base.join(dir))?;
    }
    fs::copy(root.join("schema/Ruxel.pkl"), base.join("schema/Ruxel.pkl"))?;
    let payload = base.join("payload");
    let mut source = include_str!("../fixtures/handler.pkl").to_owned();
    for (key, name) in [("FIRST", "first"), ("SECOND", "second"), ("OTHER", "other")] {
        source = source.replace(
            key,
            &serde_json::to_string(&payload.join(name).to_string_lossy())?,
        );
    }
    let module = base.join("fixtures/handler.pkl");
    fs::write(&module, source)?;
    for (index, mode) in ["apply", "apply", "check", "diff", "apply"]
        .iter()
        .enumerate()
    {
        if index == 2 {
            for name in ["first", "second"] {
                fs::write(payload.join(name), "drift")?;
            }
        }
        let before = snapshot(&payload)?;
        let failed = index != 1;
        let events = process::run_extra(
            cli,
            agent,
            &module,
            &base.join("state"),
            mode,
            Duration::from_secs(20),
            &[OsStr::new("--tags"), OsStr::new("config")],
            None,
            if failed { 8 } else { 0 },
        )?;
        validate(&events, &payload, failed)?;
        if index == 1 {
            let recap = events
                .iter()
                .find(|e| e.get("recap").is_some())
                .context("missing recap")?;
            ensure!(recap["recap"]["changed"] == 0, "converged result changed");
        }
        if matches!(*mode, "check" | "diff") {
            ensure!(snapshot(&payload)? == before, "check/diff modified payload");
        } else {
            ensure!(
                snapshot(&payload)?
                    == vec![
                        ("first".to_owned(), b"one".to_vec()),
                        ("second".to_owned(), b"two".to_vec())
                    ],
                "incorrect payload"
            );
        }
    }
    let before_other = snapshot(&payload)?;
    let events = process::run_extra(
        cli,
        agent,
        &module,
        &base.join("state"),
        "apply",
        Duration::from_secs(20),
        &[OsStr::new("--tags"), OsStr::new("other")],
        None,
        0,
    )?;
    ensure!(
        payload.join("other").is_file()
            && ["first", "second"].iter().all(|task| {
                let matching: Vec<_> = events.iter().filter(|e| e["task"] == *task).collect();
                matching.len() == 1
                    && matching[0]["result"]["skipped"] == true
                    && matching[0]["result"]["changed"] == false
            })
            && !events.iter().any(|e| e["task"] == "failed handler")
            && events
                .iter()
                .any(|e| e["task"] == "always" && e["result"]["msg"] == "always ran"),
        "other tag selection failed"
    );
    let after_other = snapshot(&payload)?;
    ensure!(
        after_other
            .iter()
            .filter(|(name, _)| name != "other")
            .eq(before_other.iter()),
        "excluded tasks modified payload"
    );
    let recaps: Vec<_> = events
        .iter()
        .filter(|e| e["host"] == "fixture" && e.get("recap").is_some())
        .collect();
    ensure!(
        recaps.len() == 1
            && recaps[0]["recap"]["failed"] == 0
            && recaps[0]["recap"]["skipped"] == 2,
        "other selection failed recap"
    );
    let before = snapshot(&payload)?;
    let invalid = process::run_extra(
        cli,
        agent,
        &module,
        &base.join("state"),
        "apply",
        Duration::from_secs(20),
        &[OsStr::new("--hosts"), OsStr::new("missing*")],
        None,
        1,
    )?;
    ensure!(
        invalid.is_empty() && snapshot(&payload)? == before,
        "unmatched host selection executed tasks"
    );
    println!("PASS failing handlers, tags and check/diff non-mutation");
    Ok(())
}

mod check;
use crate::{process, proxy};
use anyhow::{Context, Result, ensure};
use check::{check_nested, check_vars};
use std::{ffi::OsStr, fs, path::Path, time::Duration};
const SECRET: &str = "synthetic-private-loop-item-748261";
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
                "private item leaked in state"
            );
        }
    }
    Ok(())
}
fn labels(path: &Path) -> Result<Vec<String>> {
    fs::read_to_string(path)?
        .lines()
        .map(|line| serde_json::from_str(line).context("invalid label"))
        .collect()
}
pub fn run(root: &Path, cli: &Path, agent: &Path, item_vars: bool) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    for dir in ["schema", "fixtures"] {
        fs::create_dir(base.join(dir))?;
    }
    fs::copy(root.join("schema/Ruxel.pkl"), base.join("schema/Ruxel.pkl"))?;
    let source = if item_vars {
        include_str!("../fixtures/loop_task_vars.pkl")
            .replace("SECRET", &serde_json::to_string(SECRET)?)
    } else {
        include_str!("../fixtures/nested_loops.pkl").to_owned()
    };
    let module = base.join("fixtures/loops.pkl");
    fs::write(&module, source)?;
    let label_path = base.join("labels.jsonl");
    let proxy = proxy::wrapper(base, agent, &label_path)?;
    for (index, mode) in ["apply", "apply", "check", "diff"].iter().enumerate() {
        if label_path.exists() {
            fs::remove_file(&label_path)?;
        }
        let log = base.join(format!("event-{index}.jsonl"));
        let args = [OsStr::new("--log-file"), log.as_os_str()];
        let events = process::run_extra(
            cli,
            &proxy,
            &module,
            &base.join("state"),
            mode,
            Duration::from_secs(20),
            &args,
            item_vars.then_some(SECRET),
            0,
        )?;
        let log_text = fs::read_to_string(log)?;
        ensure!(
            !log_text.is_empty() && !log_text.contains(SECRET),
            "missing log or private item leaked"
        );
        let recaps: Vec<_> = events
            .iter()
            .filter(|e| e["host"] == "fixture" && e.get("recap").is_some())
            .collect();
        ensure!(
            recaps.len() == 1 && recaps[0]["recap"]["failed"] == 0,
            "{mode}: failed recap"
        );
        if item_vars {
            check_vars(&events, mode)?;
            ensure!(
                labels(&label_path)?
                    == [
                        "default-one",
                        "default-two",
                        "custom-red",
                        "custom-green",
                        "[censored]"
                    ],
                "{mode}: wrong wire labels"
            );
        } else {
            check_nested(&events, mode)?;
            ensure!(
                labels(&label_path)? == ["alpha/one", "alpha/two", "beta/three"],
                "{mode}: wrong wire labels"
            );
        }
    }
    if item_vars {
        no_secret(&base.join("state"))?;
    }
    println!(
        "PASS {} loop labels and results",
        if item_vars {
            "per-item variables"
        } else {
            "nested"
        }
    );
    Ok(())
}

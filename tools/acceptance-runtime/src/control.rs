use crate::{process, result};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path, time::Duration};
const SECRET: &str = "synthetic-flow-private-759241";
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
                "private value leaked in state"
            );
        }
    }
    Ok(())
}
pub fn run(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    for dir in ["schema", "fixtures"] {
        fs::create_dir(base.join(dir))?;
    }
    fs::copy(root.join("schema/Ruxel.pkl"), base.join("schema/Ruxel.pkl"))?;
    let marker = base.join("retry-marker");
    let quoted = format!("'{}'", marker.to_string_lossy().replace('\'', "'\\''"));
    let retry = format!("test -f {quoted} && exit 0; touch {quoted}; exit 1");
    let source = include_str!("../fixtures/control.pkl")
        .replace("RETRY", &serde_json::to_string(&retry)?)
        .replace("SECRET", &serde_json::to_string(SECRET)?);
    let module = base.join("fixtures/control.pkl");
    fs::write(&module, source)?;
    for (index, mode) in ["apply", "apply", "check", "diff"].into_iter().enumerate() {
        let before = fs::read(&marker).ok();
        let events = process::run_extra(
            cli,
            agent,
            &module,
            &base.join("state").join(index.to_string()),
            mode,
            Duration::from_secs(20),
            &[],
            Some(SECRET),
            0,
        )?;
        let recaps: Vec<_> = events
            .iter()
            .filter(|e| e["host"] == "[censored]" && e.get("recap").is_some())
            .collect();
        ensure!(
            recaps.len() == 1 && recaps[0]["recap"]["failed"] == 0,
            "{mode}: invalid recap"
        );
        ensure!(
            result(&events, "read fact")?["msg"] == "local-fact"
                && result(&events, "skip condition")?["skipped"] == true,
            "{mode}: fact/condition failed"
        );
        ensure!(
            result(&events, "rescue")?["msg"] == "rescued"
                && result(&events, "always")?["msg"] == "always ran",
            "{mode}: rescue/always failed"
        );
        ensure!(
            recaps[0]["recap"]["rescued"].as_u64().unwrap_or(0) >= 1
                && recaps[0]["recap"]["ignored"].as_u64().unwrap_or(0) >= 1,
            "{mode}: rescue/ignore not recorded"
        );
        ensure!(
            result(&events, "retry")?["rc"] == 0
                && marker.is_file()
                && result(&events, "[censored]")?["censored"] == true,
            "{mode}: retry or redaction failed"
        );
        let loop_result = result(&events, "loop")?;
        let items = loop_result["results"]
            .as_array()
            .context("missing loop results")?;
        ensure!(items.len() == 3, "{mode}: invalid iteration count");
        if matches!(mode, "check" | "diff") {
            ensure!(
                fs::read(&marker).ok() == before && items.iter().all(|v| v["skipped"] == true),
                "{mode}: mutation or missing skips"
            );
        } else {
            let env = result(&events, "environment")?;
            let overridden = result(&events, "overrides")?;
            ensure!(
                env["stdout"] == "injected-value"
                    && env["changed"] == false
                    && overridden["rc"] == 1
                    && overridden["failed"] == false
                    && overridden["changed"] == false,
                "{mode}: environment/overrides failed"
            );
            ensure!(
                items
                    .iter()
                    .zip(["one", "two", "three"])
                    .all(|(item, expected)| item["stdout"] == expected),
                "{mode}: loop result mismatch"
            );
        }
    }
    no_secret(&base.join("state"))?;
    println!("PASS local flow: facts, loops, overrides, rescue, retry and redaction");
    Ok(())
}

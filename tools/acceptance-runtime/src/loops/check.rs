use crate::result;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
pub(super) fn check_vars(events: &[Value], mode: &str) -> Result<()> {
    for (task, expected) in [
        ("default loop", ["default-one", "default-two"]),
        ("custom loop", ["custom-red", "custom-green"]),
    ] {
        let row = result(events, task)?;
        let items = row["results"].as_array().context("missing loop items")?;
        ensure!(
            items.len() == 3 && items[2]["skipped"] == true,
            "{mode}: bad conditional loop"
        );
        if matches!(mode, "check" | "diff") {
            ensure!(
                items.iter().all(|v| v["skipped"] == true),
                "{mode}: command did not skip"
            );
        } else {
            ensure!(
                items
                    .iter()
                    .take(2)
                    .zip(expected)
                    .all(|(v, s)| v["stdout"] == s),
                "{mode}: bad item vars"
            );
        }
    }
    ensure!(
        result(events, "private loop")?["censored"] == true,
        "private loop not censored"
    );
    let empty = result(events, "empty loop")?;
    ensure!(
        empty["skipped"] == true && empty["results"] == serde_json::json!([]),
        "empty loop failed"
    );
    Ok(())
}
pub(super) fn check_nested(events: &[Value], mode: &str) -> Result<()> {
    let row = result(events, "pairs")?;
    let items = row["results"].as_array().context("missing pairs")?;
    ensure!(items.len() == 3, "{mode}: invalid iteration count");
    let check = result(events, "registered assertion")?;
    if matches!(mode, "check" | "diff") {
        ensure!(
            items.iter().all(|v| v["skipped"] == true) && check["skipped"] == true,
            "{mode}: skipped result missing"
        );
    } else {
        ensure!(
            items
                .iter()
                .zip(["alpha:one", "alpha:two", "beta:three"])
                .all(|(v, s)| v["stdout"] == s)
                && check["failed"] == false,
            "{mode}: nested results incorrect"
        );
    }
    Ok(())
}

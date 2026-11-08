use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::Path;
pub(super) fn validate(events: &[Value], payload: &Path, failed: bool) -> Result<()> {
    let recaps: Vec<_> = events
        .iter()
        .filter(|e| e["host"] == "fixture" && e.get("recap").is_some())
        .collect();
    ensure!(
        recaps.len() == 1 && recaps[0]["recap"]["failed"] == u8::from(failed),
        "invalid recap"
    );
    let handlers: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "failed handler" && e.get("result").is_some())
        .collect();
    ensure!(
        handlers.len() == usize::from(failed),
        "incorrect handler count"
    );
    if failed {
        ensure!(
            handlers[0]["result"]["failed"] == true,
            "handler failure missing"
        );
    }
    let always: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "always" && e["result"]["skipped"] == true)
        .collect();
    ensure!(
        always.len() == 1 && !payload.join("other").exists(),
        "unselected tags were not skipped"
    );
    Ok(())
}

use anyhow::{Result, ensure};
use serde_json::Value;
use std::{fs, os::unix::process::ExitStatusExt, path::Path, process::ExitStatus};

pub(crate) fn verify(
    case: &str,
    status: ExitStatus,
    events: &[Value],
    marker: &Path,
    errors: &[u8],
) -> Result<()> {
    if matches!(case, "interrupt" | "ctrl-c") {
        ensure!(
            status.signal() == Some(2) || status.code() == Some(130),
            "{case}: incorrect signal exit {status}"
        );
        ensure!(!marker.exists(), "{case}: post-pause task executed");
        return Ok(());
    }
    if case == "eof" {
        ensure!(
            status.code() == Some(8),
            "{case}: expected task failure, got {status}"
        );
        ensure!(!marker.exists(), "{case}: post-pause task executed");
        ensure!(
            events.iter().any(|event| event["event"] == "host_failed"
                && event["host"] == "fixture"
                && event["failed"] == true
                && event["msg"]
                    .as_str()
                    .is_some_and(|message| message.contains("end of input")))
                && events
                    .iter()
                    .any(|event| event["host"] == "fixture" && event["recap"]["failed"] == 1),
            "{case}: missing EOF failure: {events:?}; {}",
            String::from_utf8_lossy(errors)
        );
        return Ok(());
    }
    ensure!(
        status.success(),
        "{case}: exit {status}: {}",
        String::from_utf8_lossy(errors)
    );
    let results: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "pause")
        .filter_map(|e| e.get("result"))
        .collect();
    ensure!(
        results.len() == 1,
        "{case}: missing or duplicate pause result"
    );
    let recaps: Vec<_> = events
        .iter()
        .filter(|e| e["host"] == "fixture")
        .filter_map(|e| e.get("recap"))
        .collect();
    ensure!(
        recaps.len() == 1 && recaps[0]["failed"] == 0,
        "{case}: missing or failed recap"
    );
    if matches!(case, "check" | "diff") {
        ensure!(
            results[0]["skipped"] == true && !marker.exists(),
            "{case}: preview was not skipped or mutated"
        );
    } else {
        ensure!(
            results[0]["skipped"] != true
                && results[0]["changed"] == false
                && results[0]["user_input"] == "",
            "{case}: incorrect pause result"
        );
        ensure!(
            fs::read(marker)? == b"continued",
            "{case}: incorrect continuation bytes"
        );
    }
    Ok(())
}

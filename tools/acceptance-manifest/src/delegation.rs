use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub fn verify(events: &[Value], preview: bool) -> Result<()> {
    // The frozen fixture's second task asserts the exact delegated stdout.
    // Successful apply therefore proves the value without publishing it.
    let expected = [
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":false}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":preview}}),
        json!({"host":"[censored]","recap":{"planned":0,"ok":if preview {0} else {2},"changed":0,"failed":u64::from(preview),"skipped":u64::from(preview),"rescued":0,"ignored":0}}),
    ];
    ensure!(
        events == expected,
        "delegation result, recap or censorship mismatch"
    );
    Ok(())
}

#[cfg(test)]
#[path = "delegation_tests.rs"]
mod tests;

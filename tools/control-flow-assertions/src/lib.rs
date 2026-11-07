#![forbid(unsafe_code)]
#[cfg(not(unix))]
compile_error!("file owner assertions require Unix");
mod files;
mod observations;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::Path;

/// # Errors
/// Returns an error when evidence is missing, ambiguous, unsafe or mismatched.
pub fn assert_observations(
    root: &Path,
    observations: &Value,
    events: &Value,
    outputs: &Value,
    resolve_owner: &impl Fn(&str) -> Result<u32>,
) -> Result<()> {
    let items = observations
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing control-flow observations"))?;
    let events = events
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid events"))?;
    let outputs = outputs
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("missing public output stream"))?;
    ensure!(
        outputs.len() == 3
            && ["stdout", "stderr", "log"]
                .iter()
                .all(|k| outputs.get(*k).is_some_and(Value::is_string)),
        "missing public output stream"
    );
    for item in items {
        observations::check(root, item, events, outputs, resolve_owner)?;
    }
    Ok(())
}
/// # Errors
/// Returns an error when counts are missing, non-integral, nonpositive or unequal.
pub fn assert_retry_attempts(expected: &Value, observed: &Value) -> Result<()> {
    let expected = expected
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| anyhow::anyhow!("retry attempt count missing or mismatched"))?;
    ensure!(
        observed.as_u64() == Some(expected),
        "retry attempt count missing or mismatched"
    );
    Ok(())
}
#[cfg(test)]
mod tests;

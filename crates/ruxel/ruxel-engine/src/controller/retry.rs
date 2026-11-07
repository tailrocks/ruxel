use super::{Context, Result, Value};

pub(super) fn max_attempts(task: &Value) -> Result<u64> {
    let retries = match task.get("retries") {
        None | Some(Value::Null) => 0,
        Some(value) => value.as_u64().context("retries must be nonnegative")?,
    };
    if task.get("until").is_none_or(Value::is_null) {
        return Ok(1);
    }
    retries
        .checked_add(1)
        .context("retries exceed supported range")
}

pub(super) fn delay_seconds(task: &Value) -> u64 {
    task.get("delaySeconds")
        .and_then(Value::as_u64)
        .unwrap_or(5)
}

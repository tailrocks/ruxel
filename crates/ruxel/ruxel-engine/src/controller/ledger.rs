use anyhow::{Context, Result};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

fn canonical(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        Value::Object(values) => {
            let ordered: BTreeMap<_, _> = values
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(ordered.into_iter().collect::<Map<_, _>>())
        }
        other => other,
    }
}

/// Fingerprint the material inputs to an iteration after source files are rendered.
/// No-log operations return an empty key and are never cached.
///
/// # Errors
/// Returns an error when the parameter bytes are not valid JSON.
pub fn ledger_key(task: &v1::RenderedTask, iteration: &v1::Iteration) -> Result<String> {
    if task.no_log {
        return Ok(String::new());
    }
    let params: Value =
        serde_json::from_slice(&iteration.params_json).context("invalid iteration parameters")?;
    let environment: BTreeMap<_, _> = task.environment.iter().collect();
    let source_hash = iteration
        .source_content
        .as_ref()
        .map(|bytes| blake3::hash(bytes).to_hex().to_string());
    let identity = canonical(
        json!({"version":2,"module":task.module,"name":task.name,"params":params,"freeForm":iteration.free_form,"becomeUser":task.become_user,"environment":environment,"sourceHash":source_hash}),
    );
    let bytes = serde_json::to_vec(&identity)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[cfg(test)]
#[path = "ledger/tests.rs"]
mod tests;

use crate::files;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::{fs, path::Path};
fn keys(item: &Map<String, Value>, expected: &[&str]) -> bool {
    item.len() == expected.len() && expected.iter().all(|k| item.contains_key(*k))
}
fn text<'a>(item: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    item.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("invalid {key}"))
}
fn task<'a>(events: &'a [Value], host: &Value, name: &Value) -> Result<&'a Map<String, Value>> {
    let rows: Vec<_> = events
        .iter()
        .filter(|e| {
            e.get("host") == Some(host) && e.get("task") == Some(name) && e.get("result").is_some()
        })
        .collect();
    ensure!(rows.len() == 1, "missing or duplicate result");
    let result = rows[0]["result"].as_object().context("invalid result")?;
    ensure!(
        result.get("censored") != Some(&Value::Bool(true))
            && result.get("redacted") != Some(&Value::Bool(true)),
        "result is censored"
    );
    Ok(result)
}
pub fn check(
    root: &Path,
    item: &Value,
    events: &[Value],
    outputs: &Map<String, Value>,
    resolve_owner: &impl Fn(&str) -> Result<u32>,
) -> Result<()> {
    let item = item.as_object().context("invalid observation")?;
    let kind = text(item, "kind")?;
    match kind {
        "file" => {
            ensure!(
                keys(item, &["kind", "path", "content", "owner"]),
                "invalid file observation"
            );
            let path = files::target(root, text(item, "path")?)?;
            ensure!(
                fs::read(&path)? == text(item, "content")?.as_bytes(),
                "file content mismatch"
            );
            files::owner(&path, text(item, "owner")?, resolve_owner)?;
        }
        "file_exists" => {
            ensure!(
                keys(item, &["kind", "path"]),
                "invalid file existence observation"
            );
            files::target(root, text(item, "path")?)?;
        }
        "task_result" => {
            ensure!(
                keys(item, &["kind", "host", "task", "field", "equals"]),
                "invalid task result observation"
            );
            let result = task(events, &item["host"], &item["task"])?;
            ensure!(
                result.get(text(item, "field")?) == Some(&item["equals"]),
                "task result mismatch"
            );
        }
        "task_loop_stdout" => {
            ensure!(
                keys(item, &["kind", "host", "task", "values"]),
                "invalid loop observation"
            );
            let expected = item["values"].as_array().context("invalid loop values")?;
            ensure!(expected.iter().all(Value::is_string), "invalid loop values");
            let result = task(events, &item["host"], &item["task"])?;
            let rows = result
                .get("results")
                .and_then(Value::as_array)
                .context("missing loop results")?;
            ensure!(rows.len() == expected.len(), "wrong loop cardinality");
            for (row, value) in rows.iter().zip(expected) {
                let row = row.as_object().context("invalid loop result")?;
                ensure!(
                    row.get("censored") != Some(&Value::Bool(true))
                        && row.get("redacted") != Some(&Value::Bool(true))
                        && row.get("stdout").is_some_and(Value::is_string)
                        && row.get("stdout") == Some(value),
                    "loop stdout missing, censored, or mismatched"
                );
            }
        }
        "fact" => {
            ensure!(
                keys(item, &["kind", "host", "name", "equals"]),
                "invalid fact observation"
            );
            let result = task(
                events,
                &item["host"],
                &Value::String("Record rescued synthetic state".to_owned()),
            )?;
            let facts = result
                .get("facts")
                .and_then(Value::as_object)
                .context("missing rescue facts")?;
            ensure!(
                facts.get(text(item, "name")?) == Some(&item["equals"]),
                "rescue fact mismatch"
            );
        }
        "output_excludes" => {
            ensure!(keys(item, &["kind", "values"]), "invalid output exclusion");
            let values = item["values"]
                .as_array()
                .filter(|v| !v.is_empty())
                .context("invalid output exclusion")?;
            for value in values {
                let value = value
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .context("invalid output exclusion")?;
                ensure!(
                    outputs
                        .values()
                        .all(|stream| !stream.as_str().unwrap_or_default().contains(value)),
                    "excluded value appears in public output"
                );
            }
        }
        _ => bail!("unsupported control-flow observation: {kind}"),
    }
    Ok(())
}

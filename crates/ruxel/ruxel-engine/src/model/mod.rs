//! Strict validation of evaluated Pkl, including inputs that bypass the Pkl schema.
mod operations;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    inventory: Inventory,
    plays: Vec<Play>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    hosts: BTreeMap<String, Host>,
    #[serde(default)]
    groups: BTreeMap<String, Group>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Host {
    address: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
}
fn default_port() -> u16 {
    22
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Group {
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    children: Vec<String>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Play {
    name: String,
    targets: Vec<String>,
    #[serde(default)]
    pre_tasks: Vec<Task>,
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    handlers: Vec<Task>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
    #[serde(default)]
    environment: BTreeMap<String, Value>,
    #[serde(default)]
    become_user: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Task {
    name: String,
    #[serde(default)]
    operation: Option<Value>,
    #[serde(default)]
    when: Option<Value>,
    #[serde(default, rename = "loop")]
    loop_expr: Option<Value>,
    #[serde(default = "default_loop_var")]
    loop_var: String,
    #[serde(default)]
    loop_label: Option<Value>,
    #[serde(default)]
    register: Option<String>,
    #[serde(default)]
    notify: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    delegate_to: Option<Value>,
    #[serde(default)]
    become_user: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
    #[serde(default)]
    environment: BTreeMap<String, Value>,
    #[serde(default = "default_retries")]
    retries: u64,
    #[serde(default)]
    delay_seconds: u64,
    #[serde(default)]
    until: Option<Value>,
    #[serde(default)]
    changed_when: Option<Value>,
    #[serde(default)]
    failed_when: Option<Value>,
    #[serde(default)]
    ignore_errors: bool,
    #[serde(default)]
    check_mode: Option<bool>,
    #[serde(default)]
    block: Vec<Task>,
    #[serde(default)]
    rescue: Vec<Task>,
    #[serde(default)]
    always: Vec<Task>,
    #[serde(default)]
    no_log: bool,
}
fn default_loop_var() -> String {
    "item".into()
}
fn default_retries() -> u64 {
    1
}

#[derive(Clone, Copy)]
enum FieldType {
    Text,
    Flag,
    Number,
    Port,
    Mode,
    TextList,
    ExprMap,
    Choice(&'static [&'static str]),
}
struct Field {
    name: &'static str,
    required: bool,
    kind: FieldType,
}
struct Operation {
    kind: &'static str,
    fields: &'static [Field],
}

fn allowed_fields(object: &Map<String, Value>, fields: &[&str]) -> Result<()> {
    ensure!(
        object.keys().all(|key| fields.contains(&key.as_str())),
        "unknown expression field"
    );
    Ok(())
}
fn text(value: &Value) -> bool {
    value.is_string()
}
fn text_list(value: &Value) -> bool {
    value.is_string() || value.as_array().is_some_and(|v| v.iter().all(text))
}

fn expression(value: &Value) -> Result<()> {
    let object = value.as_object().context("expected expression object")?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .context("expression requires kind")?;
    match kind {
        "literal" => {
            allowed_fields(object, &["kind", "value"])?;
            ensure!(
                object
                    .get("value")
                    .is_none_or(|v| !v.is_array() && !v.is_object()),
                "literal requires scalar"
            );
        }
        "ref" => {
            allowed_fields(object, &["kind", "path"])?;
            let path = object
                .get("path")
                .and_then(Value::as_array)
                .context("reference requires path")?;
            ensure!(
                !path.is_empty() && path.iter().all(text),
                "reference requires nonempty string path"
            );
        }
        "list" => {
            allowed_fields(object, &["kind", "items"])?;
            for item in object
                .get("items")
                .and_then(Value::as_array)
                .context("list requires items")?
            {
                expression(item)?;
            }
        }
        "map" => {
            allowed_fields(object, &["kind", "entries"])?;
            for item in object
                .get("entries")
                .and_then(Value::as_object)
                .context("map requires entries")?
                .values()
            {
                expression(item)?;
            }
        }
        "call" => validate_call(object)?,
        "lookup" => validate_lookup(object)?,
        _ => bail!("unknown expression kind"),
    }
    Ok(())
}
fn validate_call(object: &Map<String, Value>) -> Result<()> {
    allowed_fields(object, &["kind", "function", "args"])?;
    let function = object
        .get("function")
        .and_then(Value::as_str)
        .context("call requires function")?;
    let args = object
        .get("args")
        .and_then(Value::as_array)
        .context("call requires args")?;
    let arity = match function {
        "concat" => None,
        "not" | "length" | "sha256" | "base64Encode" | "base64Decode" | "bool" | "trim"
        | "urlencode" | "list" => Some(1),
        "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "and" | "or" | "default" | "contains"
        | "subelements" | "mapAttribute" | "join" => Some(2),
        _ => bail!("unknown expression function"),
    };
    ensure!(
        arity.is_none_or(|n| n == args.len()),
        "invalid expression arity"
    );
    for arg in args {
        expression(arg)?;
    }
    Ok(())
}
fn validate_lookup(object: &Map<String, Value>) -> Result<()> {
    allowed_fields(
        object,
        &[
            "kind",
            "provider",
            "key",
            "vault",
            "item",
            "section",
            "field",
            "reference",
            "argv",
        ],
    )?;
    for key in ["key", "vault", "item", "section", "field", "reference"] {
        ensure!(
            object.get(key).is_none_or(|v| v.is_null() || v.is_string()),
            "lookup selector must be text"
        );
    }
    let required = |key| {
        object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    let argv = object.get("argv").and_then(Value::as_array);
    ensure!(
        object
            .get("argv")
            .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(text))),
        "lookup argv must be string list"
    );
    match object.get("provider").and_then(Value::as_str) {
        Some("fnox" | "synthetic") => ensure!(required("key"), "lookup requires key"),
        Some("op") => ensure!(
            required("reference") || (required("vault") && required("item") && required("field")),
            "lookup requires item selectors or reference"
        ),
        Some("command") => ensure!(
            argv.is_some_and(|v| !v.is_empty()),
            "command lookup requires argv"
        ),
        Some("pipe") => ensure!(
            required("key") || argv.is_some_and(|v| v.len() == 1),
            "pipe lookup requires command"
        ),
        _ => bail!("unknown lookup provider"),
    }
    Ok(())
}

fn validate_field(value: &Value, kind: FieldType, deferred: bool) -> Result<()> {
    if value.is_object() && !matches!(kind, FieldType::ExprMap) {
        ensure!(deferred, "unresolved operation expression");
        return expression(value);
    }
    let valid = match kind {
        FieldType::Text => value.is_string(),
        FieldType::Flag => value.is_boolean(),
        FieldType::Number => value.as_u64().is_some(),
        FieldType::Port => value.as_u64().is_some_and(|n| (1..=65535).contains(&n)),
        FieldType::Mode => value.as_str().is_some_and(|s| {
            (3..=4).contains(&s.len()) && s.bytes().all(|b| (b'0'..=b'7').contains(&b))
        }),
        FieldType::TextList => text_list(value),
        FieldType::Choice(choices) => value.as_str().is_some_and(|s| choices.contains(&s)),
        FieldType::ExprMap => {
            for expr in value
                .as_object()
                .context("expected expression map")?
                .values()
            {
                if deferred {
                    expression(expr)?;
                }
            }
            true
        }
    };
    ensure!(valid, "invalid operation parameter type or value");
    Ok(())
}
fn operation(kind: &str, params: &Map<String, Value>, deferred: bool) -> Result<()> {
    let spec = operations::OPERATIONS
        .iter()
        .find(|s| s.kind == kind)
        .context("unknown operation kind")?;
    ensure!(
        params
            .keys()
            .all(|key| spec.fields.iter().any(|f| f.name == key)),
        "unknown operation parameter"
    );
    for field in spec.fields {
        match params.get(field.name).filter(|v| !v.is_null()) {
            Some(value) => validate_field(value, field.kind, deferred)?,
            None => ensure!(!field.required, "missing required operation parameter"),
        }
    }
    let present = |key| params.get(key).is_some_and(|v| !v.is_null());
    if kind == "copy" {
        ensure!(
            present("src") != present("content"),
            "copy requires exactly one of src or content"
        );
    }
    if kind == "file" {
        ensure!(
            present("path") != present("dest"),
            "file requires exactly one of path or dest"
        );
        if params.get("state").and_then(Value::as_str) == Some("link") {
            ensure!(present("src"), "link requires src");
        }
    }
    if kind == "pause" {
        ensure!(
            !(present("seconds") && present("minutes")),
            "pause accepts seconds or minutes"
        );
    }
    if matches!(kind, "command" | "shell") {
        ensure!(
            present("cmd") != present("argv"),
            "command requires exactly one of cmd or argv"
        );
        if let Some(v) = params
            .get("argv")
            .filter(|v| !v.is_null() && !v.is_object())
        {
            ensure!(
                v.as_array().is_some_and(|v| !v.is_empty()),
                "argv requires nonempty list"
            );
        }
    }
    Ok(())
}

/// Validate evaluated operation parameters before any provider executes.
///
/// # Errors
/// Rejects unknown operations, fields, invalid values and unresolved expressions.
pub fn validate_operation(kind: &str, params: &Map<String, Value>) -> Result<()> {
    operation(kind, params, false)
}

fn expression_map(values: &BTreeMap<String, Value>) -> Result<()> {
    for value in values.values() {
        expression(value)?;
    }
    Ok(())
}
fn validate_tasks(tasks: &[Task]) -> Result<()> {
    for task in tasks {
        ensure!(
            task.operation.is_some() == task.block.is_empty(),
            "task requires operation or nonempty block"
        );
        ensure!(task.retries >= 1, "retries must be positive");
        if let Some(destination) = &task.delegate_to {
            if destination.is_object() {
                expression(destination)?;
            } else {
                ensure!(
                    destination.is_string(),
                    "delegation target must be text or expression"
                );
            }
        }
        for expr in [
            &task.when,
            &task.loop_expr,
            &task.loop_label,
            &task.until,
            &task.changed_when,
            &task.failed_when,
        ]
        .into_iter()
        .flatten()
        {
            expression(expr)?;
        }
        expression_map(&task.vars)?;
        expression_map(&task.environment)?;
        if let Some(value) = &task.operation {
            let mut params = value
                .as_object()
                .context("operation must be an object")?
                .clone();
            let kind = params
                .remove("kind")
                .and_then(|v| v.as_str().map(str::to_owned))
                .context("operation requires kind")?;
            operation(&kind, &params, true)?;
        }
        validate_tasks(&task.block)?;
        validate_tasks(&task.rescue)?;
        validate_tasks(&task.always)?;
    }
    Ok(())
}
/// Validate and normalize Pkl output at the executable boundary.
///
/// # Errors
/// Rejects malformed configuration, unknown fields and invalid typed variants.
pub fn validate_config(value: &Value) -> Result<Value> {
    let value = if let Some(config) = value.get("config") {
        ensure!(
            value.as_object().is_some_and(|o| o.len() == 1),
            "unknown root output field"
        );
        config
    } else {
        value
    };
    let config: Config =
        serde_json::from_value(value.clone()).context("invalid configuration shape")?;
    for host in config.inventory.hosts.values() {
        ensure!(
            !host.address.is_empty() && host.port != 0,
            "invalid host address or port"
        );
        expression_map(&host.vars)?;
    }
    for group in config.inventory.groups.values() {
        expression_map(&group.vars)?;
    }
    for play in &config.plays {
        expression_map(&play.vars)?;
        expression_map(&play.environment)?;
        validate_tasks(&play.pre_tasks)?;
        validate_tasks(&play.tasks)?;
        validate_tasks(&play.handlers)?;
    }
    serde_json::to_value(config).context("normalize configuration")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn config(operation: Value) -> Value {
        let mut config = json!({"inventory":{"hosts":{"synthetic":{"address":"unused.invalid"}}},"plays":[{"name":"test","targets":["synthetic"],"tasks":[{"name":"test"}]}]});
        config["plays"][0]["tasks"][0]["operation"] = operation;
        config
    }

    #[test]
    fn rejects_untyped_unknown_fields_and_operation_variants() {
        assert!(validate_config(&json!({"anything":true})).is_err());
        assert!(validate_config(&config(json!({"kind":"unknown"}))).is_err());
        assert!(
            validate_config(&config(
                json!({"kind":"copy","dest":"/tmp/synthetic","content":"ok","typo":true})
            ))
            .is_err()
        );
    }
    #[test]
    fn rejects_invalid_evaluated_expression_results() {
        assert!(
            validate_operation(
                "copy",
                json!({"dest":"/tmp/synthetic","content":"ok","force":"wrong"})
                    .as_object()
                    .context("object")
                    .unwrap_or(&Map::new())
            )
            .is_err()
        );
        assert!(
            validate_operation(
                "filesystem",
                json!({"dev":"/tmp/synthetic","fstype":"btrfs"})
                    .as_object()
                    .context("object")
                    .unwrap_or(&Map::new())
            )
            .is_err()
        );
    }
    #[test]
    fn checks_nested_expression_shape_before_execution() {
        let value = config(
            json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"call","function":"unknown","args":[]}}),
        );
        assert!(validate_config(&value).is_err());
        let value = config(
            json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"ref","path":[],"typo":true}}),
        );
        assert!(validate_config(&value).is_err());
    }
    #[test]
    fn accepts_typed_deferred_values_and_plain_valid_results() -> Result<()> {
        validate_config(&config(
            json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"literal","value":"ok"}}),
        ))?;
        validate_operation(
            "copy",
            json!({"dest":"/tmp/synthetic","content":"ok","force":true})
                .as_object()
                .context("object")?,
        )?;
        Ok(())
    }
}

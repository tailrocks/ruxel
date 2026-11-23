use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};

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
pub(super) fn text_list(value: &Value) -> bool {
    value.is_string() || value.as_array().is_some_and(|v| v.iter().all(text))
}

pub(super) fn expression(value: &Value) -> Result<()> {
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
        "if" | "flatMap" => Some(3),
        "not" | "length" | "sha256" | "base64Encode" | "base64Decode" | "string" | "bool"
        | "trim" | "urlencode" | "list" => Some(1),
        "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "and" | "or" | "default" | "contains"
        | "mapAttribute" | "join" | "map" => Some(2),
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
fn validate_fnox_key(key: Option<&str>) -> Result<()> {
    ensure!(
        key.is_some_and(crate::secrets::is_fnox_key),
        "fnox lookup key is malformed: {}",
        crate::secrets::FNOX_KEY_GRAMMAR
    );
    Ok(())
}

fn validate_lookup(object: &Map<String, Value>) -> Result<()> {
    validate_lookup_fields(object)?;
    match object.get("provider").and_then(Value::as_str) {
        Some("fnox") => {
            validate_fnox_key(object.get("key").and_then(Value::as_str))?;
            ensure!(
                object.get("format").is_none_or(Value::is_null),
                "fnox lookup takes no format; a generated key names one with provider fnox-generated"
            );
        }
        Some("fnox-generated") => {
            validate_fnox_key(object.get("key").and_then(Value::as_str))?;
            ensure!(
                object
                    .get("format")
                    .and_then(Value::as_str)
                    .is_some_and(crate::secrets::is_generated_format),
                "generated lookup requires a format: hex16, hex32, base64-32 or alnum32"
            );
        }
        Some("synthetic") => validate_synthetic_lookup(object)?,
        Some("op") => validate_op_lookup(object)?,
        Some("command") => validate_command_lookup(object)?,
        _ => bail!("unknown lookup provider"),
    }
    Ok(())
}

fn validate_synthetic_lookup(object: &Map<String, Value>) -> Result<()> {
    ensure!(
        object
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| !key.is_empty()),
        "lookup requires key"
    );
    Ok(())
}

fn validate_command_lookup(object: &Map<String, Value>) -> Result<()> {
    ensure!(
        object
            .get("argv")
            .and_then(Value::as_array)
            .is_some_and(|argv| !argv.is_empty()),
        "command lookup requires argv"
    );
    Ok(())
}

fn validate_op_lookup(object: &Map<String, Value>) -> Result<()> {
    ensure!(
        object.get("reference").is_none_or(Value::is_null)
            || ["vault", "item", "section", "field"]
                .iter()
                .all(|key| object.get(*key).is_none_or(Value::is_null)),
        "lookup reference conflicts with structured selectors"
    );
    ensure!(
        object
            .get("reference")
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| !s.is_empty())),
        "lookup reference must not be empty"
    );
    let required = |key| {
        object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    ensure!(
        required("reference") || required("item"),
        "lookup requires item or reference"
    );
    if !required("reference") {
        // No defaults: the structured selector names vault, item and
        // field, every one of them.
        for key in ["vault", "item", "field"] {
            ensure!(required(key), "op lookup requires {key}");
        }
        ensure!(
            object
                .get("section")
                .is_none_or(|v| { v.is_null() || v.as_str().is_some_and(|s| !s.is_empty()) }),
            "lookup selector must not be empty"
        );
    }
    Ok(())
}

fn validate_lookup_fields(object: &Map<String, Value>) -> Result<()> {
    allowed_fields(
        object,
        &[
            "kind",
            "provider",
            "key",
            "format",
            "vault",
            "item",
            "section",
            "field",
            "reference",
            "argv",
        ],
    )?;
    for key in [
        "key",
        "format",
        "vault",
        "item",
        "section",
        "field",
        "reference",
    ] {
        ensure!(
            object.get(key).is_none_or(|v| v.is_null() || v.is_string()),
            "lookup selector must be text"
        );
    }
    ensure!(
        object
            .get("argv")
            .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(text))),
        "lookup argv must be string list"
    );
    Ok(())
}

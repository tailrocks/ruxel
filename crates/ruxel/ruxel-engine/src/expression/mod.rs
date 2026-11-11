use std::{collections::BTreeMap, fmt, fmt::Write as _};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

const URL_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub mod provenance;
mod values;
use values::{compare, equal, render_string};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    Literal {
        #[serde(default)]
        value: Value,
    },
    #[serde(rename = "ref")]
    Ref {
        path: Vec<String>,
    },
    Call {
        function: String,
        args: Vec<Expr>,
    },
    List {
        items: Vec<Expr>,
    },
    Map {
        entries: BTreeMap<String, Expr>,
    },
    Lookup {
        provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        /// The format a missing generated key is created in; set only for
        /// the `fnox-generated` provider.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vault: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        #[serde(default)]
        argv: Vec<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub facts: Value,
    pub results: Value,
    pub variables: Value,
    pub item: Value,
    pub extra: Map<String, Value>,
}

pub trait LookupResolver {
    /// # Errors
    /// Returns an error if the lookup cannot be resolved.
    fn resolve(&self, lookup: &Expr) -> Result<Value>;
}

/// # Errors
/// Returns an error for invalid expressions, missing values, or failed lookups.
pub fn evaluate(expr: &Expr, scope: &Scope, resolver: &impl LookupResolver) -> Result<Value> {
    match expr {
        Expr::Literal { value } => Ok(value.clone()),
        Expr::Ref { path } => reference(path, scope).cloned(),
        Expr::List { items } => items.iter().map(|e| evaluate(e, scope, resolver)).collect(),
        Expr::Map { entries } => entries
            .iter()
            .map(|(k, e)| Ok((k.clone(), evaluate(e, scope, resolver)?)))
            .collect::<Result<Map<_, _>>>()
            .map(Value::Object),
        Expr::Lookup { .. } => resolver.resolve(expr),
        Expr::Call { function, args } => call(function, args, scope, resolver),
    }
}

#[derive(Debug)]
struct MissingReference(String);
impl fmt::Display for MissingReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing reference {}", self.0)
    }
}
impl std::error::Error for MissingReference {}

fn reference<'a>(path: &[String], scope: &'a Scope) -> Result<&'a Value> {
    let (root, tail) = path.split_first().context("empty reference")?;
    let mut value = match root.as_str() {
        "facts" => &scope.facts,
        "results" => &scope.results,
        "vars" => &scope.variables,
        "item" => &scope.item,
        _ => scope
            .extra
            .get(root)
            .ok_or_else(|| MissingReference(root.clone()))?,
    };
    for segment in tail {
        value = match value {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            _ => None,
        }
        .ok_or_else(|| MissingReference(path.join(".")))?;
    }
    Ok(value)
}

fn call(
    function: &str,
    args: &[Expr],
    scope: &Scope,
    resolver: &impl LookupResolver,
) -> Result<Value> {
    match function {
        "concat" => {}
        "if" | "flatMap" => arity(function, args, 3)?,
        "not" | "length" | "sha256" | "base64Encode" | "base64Decode" | "bool" | "trim"
        | "urlencode" | "list" | "string" => arity(function, args, 1)?,
        "default" | "and" | "or" | "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains"
        | "mapAttribute" | "join" | "map" => arity(function, args, 2)?,
        _ => bail!("unknown expression function {function}"),
    }
    match function {
        "if" => {
            let condition = boolean(&evaluate(&args[0], scope, resolver)?)?;
            return evaluate(&args[if condition { 1 } else { 2 }], scope, resolver);
        }
        "flatMap" => {
            let source = evaluate(&args[0], scope, resolver)?;
            let items = source.as_array().context("flatMap expects a list")?;
            let binding = evaluate(&args[1], scope, resolver)?;
            let binding = binding.as_str().context("flatMap expects a binding name")?;
            ensure!(valid_flat_map_binding(binding), "invalid flatMap binding");
            let mut child = scope.clone();
            let mut result = Vec::new();
            for item in items {
                child.extra.insert(binding.to_owned(), item.clone());
                let value = evaluate(&args[2], &child, resolver)?;
                result.extend(
                    value
                        .as_array()
                        .context("flatMap body must return a list")?
                        .iter()
                        .cloned(),
                );
            }
            return Ok(Value::Array(result));
        }
        "map" => {
            let source = evaluate(&args[0], scope, resolver)?;
            let items = source.as_array().context("map expects a list")?;
            let mut child = scope.clone();
            return items
                .iter()
                .map(|item| {
                    child.item = item.clone();
                    evaluate(&args[1], &child, resolver)
                })
                .collect::<Result<Vec<_>>>()
                .map(Value::Array);
        }
        "default" => {
            arity(function, args, 2)?;
            return match evaluate(&args[0], scope, resolver) {
                Ok(Value::Null) => evaluate(&args[1], scope, resolver),
                Ok(value) => Ok(value),
                Err(error) if is_missing_reference(&error) => evaluate(&args[1], scope, resolver),
                Err(error) => Err(error),
            };
        }
        "and" | "or" => {
            arity(function, args, 2)?;
            let left = boolean(&evaluate(&args[0], scope, resolver)?)?;
            if (function == "and" && !left) || (function == "or" && left) {
                return Ok(json!(left));
            }
            return Ok(json!(boolean(&evaluate(&args[1], scope, resolver)?)?));
        }
        _ => {}
    }
    let values = args
        .iter()
        .map(|e| evaluate(e, scope, resolver))
        .collect::<Result<Vec<_>>>()?;
    evaluate_call(function, args, &values)
}

fn valid_flat_map_binding(binding: &str) -> bool {
    !binding.is_empty()
        && ![
            "item",
            "vars",
            "facts",
            "results",
            "inventoryHost",
            "setFacts",
        ]
        .contains(&binding)
}

fn evaluate_call(function: &str, args: &[Expr], values: &[Value]) -> Result<Value> {
    match function {
        "not" => {
            arity(function, args, 1)?;
            Ok(json!(!boolean(&values[0])?))
        }
        "eq" | "ne" => {
            arity(function, args, 2)?;
            Ok(json!(equal(&values[0], &values[1])? == (function == "eq")))
        }
        "lt" | "le" | "gt" | "ge" => {
            arity(function, args, 2)?;
            let ordering = compare(&values[0], &values[1])?;
            Ok(json!(match function {
                "lt" => ordering.is_lt(),
                "le" => !ordering.is_gt(),
                "gt" => ordering.is_gt(),
                _ => !ordering.is_lt(),
            }))
        }
        "contains" => {
            arity(function, args, 2)?;
            Ok(json!(match (&values[0], &values[1]) {
                (Value::String(haystack), Value::String(needle)) => haystack.contains(needle),
                (Value::Array(items), needle) => {
                    let mut found = false;
                    for item in items {
                        if equal(item, needle)? {
                            found = true;
                            break;
                        }
                    }
                    found
                }
                (Value::Object(map), Value::String(key)) => map.contains_key(key),
                _ => bail!("contains expects a string, list, or map and a compatible value"),
            }))
        }
        "length" => {
            arity(function, args, 1)?;
            Ok(json!(match &values[0] {
                Value::String(s) => s.chars().count(),
                Value::Array(v) => v.len(),
                Value::Object(v) => v.len(),
                _ => bail!("length expects a string, list, or map"),
            }))
        }
        "concat" => {
            let mut result = String::new();
            for value in values {
                result.push_str(value.as_str().context("concat expects strings")?);
            }
            Ok(json!(result))
        }
        "sha256" => {
            arity(function, args, 1)?;
            let text = values[0].as_str().context("sha256 expects a string")?;
            let mut digest = String::new();
            for byte in Sha256::digest(text.as_bytes()) {
                write!(&mut digest, "{byte:02x}")?;
            }
            Ok(json!(digest))
        }
        "base64Encode" => {
            arity(function, args, 1)?;
            let text = values[0]
                .as_str()
                .context("base64Encode expects a string")?;
            Ok(json!(STANDARD.encode(text)))
        }
        "base64Decode" => {
            arity(function, args, 1)?;
            let text = values[0]
                .as_str()
                .context("base64Decode expects a string")?;
            Ok(json!(String::from_utf8(STANDARD.decode(text.trim())?)?))
        }
        _ => evaluate_transform(function, args, values),
    }
}

fn evaluate_transform(function: &str, args: &[Expr], values: &[Value]) -> Result<Value> {
    match function {
        "string" => Ok(json!(render_string(&values[0]))),
        "bool" => Ok(json!(match &values[0] {
            Value::Bool(value) => *value,
            Value::String(value) => match value.as_str() {
                "true" => true,
                "false" => false,
                _ => bail!("bool expects a boolean or canonical boolean string"),
            },
            _ => bail!("bool expects a boolean or canonical boolean string"),
        })),
        "trim" => Ok(json!(
            values[0].as_str().context("trim expects a string")?.trim()
        )),
        "urlencode" => Ok(json!(
            utf8_percent_encode(
                values[0].as_str().context("urlencode expects a string")?,
                URL_ENCODE_SET
            )
            .to_string()
        )),
        "list" => match &values[0] {
            Value::Array(_) => Ok(values[0].clone()),
            Value::String(value) => Ok(json!(
                value.chars().map(|c| c.to_string()).collect::<Vec<_>>()
            )),
            _ => bail!("list expects a list or string"),
        },
        "join" => {
            let items = values[0].as_array().context("join expects a list")?;
            let separator = values[1]
                .as_str()
                .context("join expects a separator string")?;
            Ok(json!(
                items
                    .iter()
                    .map(|item| item.as_str().context("join expects strings"))
                    .collect::<Result<Vec<_>>>()?
                    .join(separator)
            ))
        }
        "mapAttribute" => {
            arity(function, args, 2)?;
            map_attribute(&values[0], &values[1])
        }

        _ => bail!("unknown expression function {function}"),
    }
}

fn arity(function: &str, args: &[Expr], expected: usize) -> Result<()> {
    ensure!(
        args.len() == expected,
        "{function} expects {expected} arguments, got {}",
        args.len()
    );
    Ok(())
}
fn boolean(value: &Value) -> Result<bool> {
    value.as_bool().context("expected boolean")
}
fn is_missing_reference(error: &anyhow::Error) -> bool {
    error.downcast_ref::<MissingReference>().is_some()
}
fn map_attribute(items: &Value, key: &Value) -> Result<Value> {
    let items = items.as_array().context("mapAttribute expects a list")?;
    let key = key.as_str().context("mapAttribute expects a field name")?;
    items
        .iter()
        .map(|item| {
            let mut value = item;
            for part in key.split('.') {
                value = value
                    .get(part)
                    .with_context(|| format!("missing mapAttribute field {key}"))?;
            }
            Ok(value.clone())
        })
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}
#[cfg(test)]
mod tests;

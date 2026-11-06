use std::{collections::BTreeMap, fmt};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// An expression evaluated against a host's current execution scope.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    Literal {
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

/// Resolve external lookups at the boundary; implementations must avoid logging secrets.
pub trait LookupResolver {
    fn resolve(&self, lookup: &Expr) -> Result<Value>;
}

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
        "variables" | "vars" => &scope.variables,
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
    // These operators must not evaluate an unused branch, which may reference absent data.
    match function {
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
    match function {
        "not" => {
            arity(function, args, 1)?;
            Ok(json!(!boolean(&values[0])?))
        }
        "eq" | "ne" => {
            arity(function, args, 2)?;
            Ok(json!((values[0] == values[1]) == (function == "eq")))
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
                (Value::Array(items), needle) => items.contains(needle),
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
            for value in &values {
                result.push_str(value.as_str().context("concat expects strings")?);
            }
            Ok(json!(result))
        }
        "sha256" => {
            arity(function, args, 1)?;
            let text = values[0].as_str().context("sha256 expects a string")?;
            Ok(json!(
                Sha256::digest(text.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ))
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
            Ok(json!(String::from_utf8(STANDARD.decode(text)?)?))
        }
        "subelements" => {
            arity(function, args, 2)?;
            subelements(&values[0], &values[1])
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
fn compare(left: &Value, right: &Value) -> Result<std::cmp::Ordering> {
    match (left, right) {
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .zip(b.as_f64())
            .and_then(|(a, b)| a.partial_cmp(&b))
            .context("numbers cannot be compared"),
        _ => bail!("comparison expects two strings or two numbers"),
    }
}
fn is_missing_reference(error: &anyhow::Error) -> bool {
    error.downcast_ref::<MissingReference>().is_some()
}
fn subelements(parents: &Value, key: &Value) -> Result<Value> {
    let parents = parents.as_array().context("subelements expects a list")?;
    let key = key.as_str().context("subelements expects a field name")?;
    let mut result = Vec::new();
    for parent in parents {
        let mut children = parent;
        for part in key.split('.') {
            children = children
                .get(part)
                .with_context(|| format!("missing subelements field {key}"))?;
        }
        for child in children
            .as_array()
            .context("subelements field must be a list")?
        {
            result.push(json!([parent, child]));
        }
    }
    Ok(Value::Array(result))
}

#[cfg(test)]
mod tests;

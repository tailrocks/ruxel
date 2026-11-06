//! Secret provenance for runtime values. This module does not alter execution by itself.

use super::{Expr, LookupResolver, MissingReference, boolean, evaluate_call, is_missing_reference};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Sensitivity associated with a value or individual children of a collection.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Provenance {
    #[default]
    Public,
    Secret,
    Array(Vec<Provenance>),
    Object(BTreeMap<String, Provenance>),
}

impl Provenance {
    /// Whether this value or any descendant depends on a secret.
    #[must_use]
    pub fn is_secret(&self) -> bool {
        match self {
            Self::Public => false,
            Self::Secret => true,
            Self::Array(items) => items.iter().any(Self::is_secret),
            Self::Object(items) => items.values().any(Self::is_secret),
        }
    }

    fn child(&self, key: &str) -> Self {
        match self {
            Self::Public => Self::Public,
            Self::Secret => Self::Secret,
            Self::Array(items) => key
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .cloned()
                .unwrap_or(Self::Secret),
            Self::Object(items) => items.get(key).cloned().unwrap_or_default(),
        }
    }
}

/// A JSON value with independent provenance for its collection members.
#[derive(Clone, PartialEq)]
pub struct TrackedValue {
    pub value: Value,
    pub provenance: Provenance,
}

impl std::fmt::Debug for TrackedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackedValue")
            .field("value", &self.redacted())
            .field("provenance", &self.provenance)
            .finish()
    }
}

impl TrackedValue {
    #[must_use]
    pub fn public(value: Value) -> Self {
        Self {
            value,
            provenance: Provenance::Public,
        }
    }
    #[must_use]
    pub fn secret(value: Value) -> Self {
        Self {
            value,
            provenance: Provenance::Secret,
        }
    }
    #[must_use]
    pub fn is_secret(&self) -> bool {
        self.provenance.is_secret()
    }

    /// Construct a tracked object from independently tracked fields.
    #[must_use]
    pub fn object(fields: BTreeMap<String, Self>) -> Self {
        Self {
            value: Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.value.clone()))
                    .collect(),
            ),
            provenance: Provenance::Object(
                fields.into_iter().map(|(k, v)| (k, v.provenance)).collect(),
            ),
        }
    }

    /// Retrieve a child without discarding its provenance.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Self> {
        let value = match &self.value {
            Value::Object(fields) => fields.get(key),
            Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
            _ => None,
        }?;
        Some(Self {
            value: value.clone(),
            provenance: self.provenance.child(key),
        })
    }

    /// Replace a field in a tracked object, preserving sensitivity of siblings.
    ///
    /// # Errors
    /// Returns an error when this value or its provenance is not an object.
    pub fn insert(&mut self, key: String, value: Self) -> Result<()> {
        let fields = self
            .value
            .as_object_mut()
            .context("tracked value must be an object")?;
        match &mut self.provenance {
            Provenance::Public => {
                self.provenance =
                    Provenance::Object(BTreeMap::from([(key.clone(), value.provenance)]));
            }
            Provenance::Secret => {}
            Provenance::Object(tree) => {
                tree.insert(key.clone(), value.provenance);
            }
            Provenance::Array(_) => bail!("object has incompatible provenance"),
        }
        fields.insert(key, value.value);
        Ok(())
    }

    /// Redact sensitive nodes while retaining public siblings and their structure.
    #[must_use]
    pub fn redacted(&self) -> Value {
        fn redact(value: &Value, provenance: &Provenance) -> Value {
            match (value, provenance) {
                (Value::Array(values), Provenance::Array(items)) if values.len() == items.len() => {
                    Value::Array(
                        values
                            .iter()
                            .zip(items)
                            .map(|(v, p)| redact(v, p))
                            .collect(),
                    )
                }
                (Value::Object(values), Provenance::Object(items)) => Value::Object(
                    values
                        .iter()
                        .map(|(key, v)| {
                            (
                                key.clone(),
                                redact(v, items.get(key).unwrap_or(&Provenance::Public)),
                            )
                        })
                        .collect(),
                ),
                (_, Provenance::Public) => value.clone(),
                // A mismatched tree cannot safely be treated as public.
                _ => Value::String("[REDACTED]".into()),
            }
        }
        redact(&self.value, &self.provenance)
    }
}

#[derive(Debug)]
struct SensitiveMissingReference;
impl std::fmt::Display for SensitiveMissingReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sensitive reference missing")
    }
}
impl std::error::Error for SensitiveMissingReference {}

fn reference(path: &[String], scope: &TrackedValue) -> Result<TrackedValue> {
    let (root, rest) = path.split_first().context("empty reference")?;
    let root = if root == "variables" {
        "vars"
    } else {
        root.as_str()
    };
    let mut value = &scope.value;
    let mut provenance = scope.provenance.clone();
    for part in std::iter::once(root).chain(rest.iter().map(String::as_str)) {
        let next = match value {
            Value::Object(map) => map.get(part),
            Value::Array(items) => part
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            _ => None,
        };
        value = match next {
            Some(value) => value,
            None if provenance.is_secret() => return Err(SensitiveMissingReference.into()),
            None => return Err(MissingReference(path.join(".")).into()),
        };
        provenance = provenance.child(part);
    }
    Ok(TrackedValue {
        value: value.clone(),
        provenance,
    })
}

/// Evaluate an expression with provenance. Scope is an object containing the
/// same roots as the controller scope (`vars`, `facts`, `results`, and `item`).
/// The caller must carry the returned provenance with the value.
///
/// # Errors
/// Returns an error for an invalid expression or failed lookup. Errors from
/// operations consuming sensitive values are replaced with a generic message.
pub fn evaluate_tracked(
    expr: &Expr,
    scope: &TrackedValue,
    resolver: &impl LookupResolver,
) -> Result<TrackedValue> {
    match expr {
        Expr::Literal { value } => Ok(TrackedValue::public(value.clone())),
        Expr::Ref { path } => reference(path, scope),
        Expr::Lookup { .. } => resolver
            .resolve(expr)
            .map(TrackedValue::secret)
            .map_err(|_| anyhow::anyhow!("secret lookup failed")),
        Expr::List { items } => {
            let items = items
                .iter()
                .map(|e| evaluate_tracked(e, scope, resolver))
                .collect::<Result<Vec<_>>>()?;
            Ok(TrackedValue {
                value: Value::Array(items.iter().map(|v| v.value.clone()).collect()),
                provenance: Provenance::Array(items.into_iter().map(|v| v.provenance).collect()),
            })
        }
        Expr::Map { entries } => {
            let entries = entries
                .iter()
                .map(|(k, e)| Ok((k.clone(), evaluate_tracked(e, scope, resolver)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            Ok(TrackedValue {
                value: Value::Object(
                    entries
                        .iter()
                        .map(|(k, v)| (k.clone(), v.value.clone()))
                        .collect::<Map<_, _>>(),
                ),
                provenance: Provenance::Object(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k, v.provenance))
                        .collect(),
                ),
            })
        }
        Expr::Call { function, args } => call(function, args, scope, resolver),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "lazy operators need distinct provenance handling"
)]
fn call(
    function: &str,
    args: &[Expr],
    scope: &TrackedValue,
    resolver: &impl LookupResolver,
) -> Result<TrackedValue> {
    let expected = match function {
        "concat" => None,
        "if" => Some(3),
        "not" | "length" | "sha256" | "base64Encode" | "base64Decode" | "bool" | "trim"
        | "urlencode" | "list" | "string" => Some(1),
        "default" | "and" | "or" | "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains"
        | "mapAttribute" | "subelements" | "join" | "map" => Some(2),
        _ => bail!("unknown expression function {function}"),
    };
    if let Some(expected) = expected {
        super::arity(function, args, expected)?;
    }
    match function {
        "if" => {
            let condition = evaluate_tracked(&args[0], scope, resolver)?;
            let selected =
                boolean(&condition.value).map_err(|e| sensitive_error(e, condition.is_secret()))?;
            let mut result = evaluate_tracked(&args[if selected { 1 } else { 2 }], scope, resolver)
                .map_err(|e| sensitive_error(e, condition.is_secret()))?;
            if condition.is_secret() {
                result.provenance = Provenance::Secret;
            }
            return Ok(result);
        }
        "default" => {
            return match evaluate_tracked(&args[0], scope, resolver) {
                Ok(value) if value.value.is_null() => {
                    let mut fallback = evaluate_tracked(&args[1], scope, resolver)
                        .map_err(|e| sensitive_error(e, value.is_secret()))?;
                    if value.is_secret() {
                        fallback.provenance = Provenance::Secret;
                    }
                    Ok(fallback)
                }
                Ok(value) => Ok(value),
                Err(error) if error.downcast_ref::<SensitiveMissingReference>().is_some() => {
                    let mut fallback = evaluate_tracked(&args[1], scope, resolver)
                        .map_err(|e| sensitive_error(e, true))?;
                    fallback.provenance = Provenance::Secret;
                    Ok(fallback)
                }
                Err(error) if is_missing_reference(&error) => {
                    evaluate_tracked(&args[1], scope, resolver)
                }
                Err(error) => Err(error),
            };
        }
        "and" | "or" => {
            let left = evaluate_tracked(&args[0], scope, resolver)?;
            let b = boolean(&left.value).map_err(|e| sensitive_error(e, left.is_secret()))?;
            if (function == "and" && !b) || (function == "or" && b) {
                return Ok(if left.is_secret() {
                    TrackedValue::secret(Value::Bool(b))
                } else {
                    TrackedValue::public(Value::Bool(b))
                });
            }
            let right = evaluate_tracked(&args[1], scope, resolver)
                .map_err(|e| sensitive_error(e, left.is_secret()))?;
            let sensitive = left.is_secret() || right.is_secret();
            let value =
                Value::Bool(boolean(&right.value).map_err(|e| sensitive_error(e, sensitive))?);
            return Ok(if sensitive {
                TrackedValue::secret(value)
            } else {
                TrackedValue::public(value)
            });
        }
        "map" => {
            let source = evaluate_tracked(&args[0], scope, resolver)?;
            let items = source
                .value
                .as_array()
                .context("map expects a list")
                .map_err(|e| sensitive_error(e, source.is_secret()))?;
            let mut result = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut child = scope.clone();
                let obj = child
                    .value
                    .as_object_mut()
                    .context("scope must be an object")?;
                obj.insert("item".into(), item.clone());
                let mut tree = match child.provenance {
                    Provenance::Public => BTreeMap::new(),
                    Provenance::Object(v) => v,
                    Provenance::Secret => {
                        result.push(
                            evaluate_tracked(&args[1], &child, resolver)
                                .map_err(|e| sensitive_error(e, true))?,
                        );
                        continue;
                    }
                    Provenance::Array(_) => bail!("scope provenance must be an object"),
                };
                tree.insert("item".into(), source.provenance.child(&index.to_string()));
                child.provenance = Provenance::Object(tree);
                result.push(
                    evaluate_tracked(&args[1], &child, resolver)
                        .map_err(|e| sensitive_error(e, source.is_secret()))?,
                );
            }
            return Ok(TrackedValue {
                value: Value::Array(result.iter().map(|v| v.value.clone()).collect()),
                provenance: if source.provenance == Provenance::Secret {
                    Provenance::Secret
                } else {
                    Provenance::Array(result.into_iter().map(|v| v.provenance).collect())
                },
            });
        }
        _ => {}
    }
    let mut values = Vec::new();
    let mut sensitive = false;
    for arg in args {
        let value =
            evaluate_tracked(arg, scope, resolver).map_err(|e| sensitive_error(e, sensitive))?;
        sensitive |= value.is_secret();
        values.push(value.value);
    }
    let value =
        evaluate_call(function, args, &values).map_err(|e| sensitive_error(e, sensitive))?;
    Ok(if sensitive {
        TrackedValue::secret(value)
    } else {
        TrackedValue::public(value)
    })
}

fn sensitive_error(error: anyhow::Error, sensitive: bool) -> anyhow::Error {
    if sensitive {
        anyhow::anyhow!("sensitive expression failed")
    } else {
        error
    }
}

#[cfg(test)]
mod tests;

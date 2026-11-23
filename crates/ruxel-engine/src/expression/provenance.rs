use super::{Expr, LookupResolver, MissingReference, boolean, evaluate_call, is_missing_reference};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod value;
pub use value::{Provenance, TrackedValue};

#[derive(Debug)]
struct SensitiveMissingReference;
impl std::fmt::Display for SensitiveMissingReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sensitive reference missing")
    }
}
impl std::error::Error for SensitiveMissingReference {}

pub(crate) fn copy_evaluation_error(error: &anyhow::Error) -> anyhow::Error {
    if let Some(missing) = error.downcast_ref::<MissingReference>() {
        MissingReference(missing.0.clone()).into()
    } else if error.downcast_ref::<SensitiveMissingReference>().is_some() {
        SensitiveMissingReference.into()
    } else {
        anyhow::anyhow!(error.to_string())
    }
}

fn reference(path: &[String], scope: &TrackedValue) -> Result<TrackedValue> {
    let (root, rest) = path.split_first().context("empty reference")?;
    let root = root.as_str();
    let mut value = &scope.value;
    let mut provenance = scope.provenance.validated(&scope.value);
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

pub(crate) trait VariableResolver {
    fn resolve(&self, path: &[String], scope: &TrackedValue) -> Result<Option<TrackedValue>>;
}

struct NoVariables;
impl VariableResolver for NoVariables {
    fn resolve(&self, _path: &[String], _scope: &TrackedValue) -> Result<Option<TrackedValue>> {
        Ok(None)
    }
}

/// # Errors
/// Returns an error for an invalid expression or failed lookup. Errors from
/// operations consuming sensitive values are replaced with a generic message.
pub fn evaluate_tracked(
    expr: &Expr,
    scope: &TrackedValue,
    resolver: &impl LookupResolver,
) -> Result<TrackedValue> {
    evaluate_with_variables(expr, scope, resolver, &NoVariables)
}

pub(crate) fn evaluate_with_variables(
    expr: &Expr,
    scope: &TrackedValue,
    resolver: &impl LookupResolver,
    variables: &dyn VariableResolver,
) -> Result<TrackedValue> {
    match expr {
        Expr::Literal { value } => Ok(TrackedValue::public(value.clone())),
        Expr::Ref { path } => match variables.resolve(path, scope)? {
            Some(value) => Ok(value),
            None => reference(path, scope),
        },
        Expr::Lookup { .. } => resolver
            .resolve(expr)
            .map(TrackedValue::secret)
            .map_err(|_| anyhow::anyhow!("secret lookup failed")),
        Expr::List { items } => {
            let items = items
                .iter()
                .map(|e| evaluate_with_variables(e, scope, resolver, variables))
                .collect::<Result<Vec<_>>>()?;
            Ok(TrackedValue {
                value: Value::Array(items.iter().map(|v| v.value.clone()).collect()),
                provenance: Provenance::Array(items.into_iter().map(|v| v.provenance).collect()),
            })
        }
        Expr::Map { entries } => {
            let entries = entries
                .iter()
                .map(|(k, e)| {
                    Ok((
                        k.clone(),
                        evaluate_with_variables(e, scope, resolver, variables)?,
                    ))
                })
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
        Expr::Call { function, args } => call(function, args, scope, resolver, variables),
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
    variables: &dyn VariableResolver,
) -> Result<TrackedValue> {
    let expected = match function {
        "concat" => None,
        "if" | "flatMap" => Some(3),
        "not" | "length" | "sha256" | "base64Encode" | "base64Decode" | "bool" | "trim"
        | "urlencode" | "list" | "string" => Some(1),
        "default" | "and" | "or" | "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains"
        | "mapAttribute" | "join" | "map" => Some(2),
        _ => bail!("unknown expression function {function}"),
    };
    if let Some(expected) = expected {
        super::arity(function, args, expected)?;
    }
    match function {
        "if" => {
            let condition = evaluate_with_variables(&args[0], scope, resolver, variables)?;
            let selected =
                boolean(&condition.value).map_err(|e| sensitive_error(e, condition.is_secret()))?;
            let mut result = evaluate_with_variables(
                &args[if selected { 1 } else { 2 }],
                scope,
                resolver,
                variables,
            )
            .map_err(|e| sensitive_error(e, condition.is_secret()))?;
            if condition.is_secret() {
                result.provenance = Provenance::Secret;
            }
            return Ok(result);
        }
        "default" => {
            return match evaluate_with_variables(&args[0], scope, resolver, variables) {
                Ok(value) if value.value.is_null() => {
                    let mut fallback =
                        evaluate_with_variables(&args[1], scope, resolver, variables)
                            .map_err(|e| sensitive_error(e, value.is_secret()))?;
                    if value.is_secret() {
                        fallback.provenance = Provenance::Secret;
                    }
                    Ok(fallback)
                }
                Ok(value) => Ok(value),
                Err(error) if error.downcast_ref::<SensitiveMissingReference>().is_some() => {
                    let mut fallback =
                        evaluate_with_variables(&args[1], scope, resolver, variables)
                            .map_err(|e| sensitive_error(e, true))?;
                    fallback.provenance = Provenance::Secret;
                    Ok(fallback)
                }
                Err(error) if is_missing_reference(&error) => {
                    evaluate_with_variables(&args[1], scope, resolver, variables)
                }
                Err(error) => Err(error),
            };
        }
        "and" | "or" => {
            let left = evaluate_with_variables(&args[0], scope, resolver, variables)?;
            let b = boolean(&left.value).map_err(|e| sensitive_error(e, left.is_secret()))?;
            if (function == "and" && !b) || (function == "or" && b) {
                return Ok(if left.is_secret() {
                    TrackedValue::secret(Value::Bool(b))
                } else {
                    TrackedValue::public(Value::Bool(b))
                });
            }
            let right = evaluate_with_variables(&args[1], scope, resolver, variables)
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
        "flatMap" => {
            let mut source = evaluate_with_variables(&args[0], scope, resolver, variables)?;
            source.provenance = source.provenance.validated(&source.value);
            let binding = evaluate_with_variables(&args[1], scope, resolver, variables)?;
            let sensitive = source.is_secret() || binding.is_secret();
            let name = binding
                .value
                .as_str()
                .context("flatMap expects a binding name")
                .map_err(|e| sensitive_error(e, sensitive))?;
            if !super::valid_flat_map_binding(name) {
                return Err(sensitive_error(
                    anyhow::anyhow!("invalid flatMap binding"),
                    sensitive,
                ));
            }
            let items = source
                .value
                .as_array()
                .context("flatMap expects a list")
                .map_err(|e| sensitive_error(e, sensitive))?;
            let mut values = Vec::new();
            let mut provenance = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut child = scope.clone();
                child
                    .value
                    .as_object_mut()
                    .context("scope must be an object")?
                    .insert(name.to_owned(), item.clone());
                let mut tree = match child.provenance {
                    Provenance::Public | Provenance::Secret => BTreeMap::new(),
                    Provenance::Object(v) => v,
                    Provenance::Array(_) => bail!("scope provenance must be an object"),
                };
                if scope.provenance == Provenance::Secret {
                    child.provenance = Provenance::Secret;
                } else {
                    tree.insert(name.to_owned(), source.provenance.child(&index.to_string()));
                    child.provenance = Provenance::Object(tree);
                }
                let result = evaluate_with_variables(&args[2], &child, resolver, variables)
                    .map_err(|e| sensitive_error(e, sensitive))?;
                let result_provenance = result.provenance.validated(&result.value);
                let list = result
                    .value
                    .as_array()
                    .context("flatMap body must return a list")
                    .map_err(|e| sensitive_error(e, sensitive || result.is_secret()))?;
                for (i, value) in list.iter().enumerate() {
                    values.push(value.clone());
                    provenance.push(result_provenance.child(&i.to_string()));
                }
            }
            return Ok(TrackedValue {
                value: Value::Array(values),
                provenance: if source.provenance == Provenance::Secret || binding.is_secret() {
                    Provenance::Secret
                } else {
                    Provenance::Array(provenance)
                },
            });
        }
        "map" => {
            let mut source = evaluate_with_variables(&args[0], scope, resolver, variables)?;
            source.provenance = source.provenance.validated(&source.value);
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
                            evaluate_with_variables(&args[1], &child, resolver, variables)
                                .map_err(|e| sensitive_error(e, true))?,
                        );
                        continue;
                    }
                    Provenance::Array(_) => bail!("scope provenance must be an object"),
                };
                tree.insert("item".into(), source.provenance.child(&index.to_string()));
                child.provenance = Provenance::Object(tree);
                result.push(
                    evaluate_with_variables(&args[1], &child, resolver, variables)
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
        let value = evaluate_with_variables(arg, scope, resolver, variables)
            .map_err(|e| sensitive_error(e, sensitive))?;
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

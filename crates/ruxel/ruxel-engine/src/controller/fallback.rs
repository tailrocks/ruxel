use super::{Backend, Map, Result, TrackedValue, Value, loop_vars};
use std::collections::BTreeMap;

pub(super) fn evaluate_layers<B: Backend + ?Sized>(
    backend: &mut B,
    expression: &Value,
    scope: &TrackedValue,
    layers: &[Map<String, Value>],
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    let Some((last, parents)) = layers.split_last() else {
        return Ok((
            backend.evaluate_tracked(expression, scope)?,
            BTreeMap::new(),
        ));
    };
    let mut scope = scope.clone();
    let mut resolved = BTreeMap::new();
    for layer in parents {
        let (_, values) = backend.evaluate_task_expression(
            &serde_json::json!({"kind":"ref","path":["vars"]}),
            &scope,
            layer,
        )?;
        let mut vars = scope
            .get("vars")
            .unwrap_or_else(|| TrackedValue::public(serde_json::json!({})));
        for (name, value) in values {
            vars.insert(name.clone(), value.clone())?;
            resolved.insert(name, value);
        }
        scope.insert("vars".into(), vars)?;
    }
    let (result, values) = backend.evaluate_task_expression(expression, &scope, last)?;
    resolved.extend(values);
    Ok((result, resolved))
}

pub(super) fn evaluate<B: Backend + ?Sized>(
    backend: &mut B,
    expression: &Value,
    scope: &TrackedValue,
    vars: &Map<String, Value>,
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    let required = loop_vars::required(vars, expression);
    let mut pending: Vec<_> = vars
        .iter()
        .filter(|(name, _)| required.contains(*name))
        .collect();
    let mut scope = scope.clone();
    let mut resolved = BTreeMap::new();
    while !pending.is_empty() {
        let previous = pending.len();
        let mut unresolved = Vec::new();
        let mut error = None;
        for (name, definition) in pending {
            if loop_vars::required(vars, definition)
                .iter()
                .any(|dependency| {
                    dependency != name
                        && vars.contains_key(dependency)
                        && !resolved.contains_key(dependency)
                })
            {
                unresolved.push((name, definition));
                continue;
            }
            match backend.evaluate_tracked(definition, &scope) {
                Ok(value) => {
                    let mut bindings = scope
                        .get("vars")
                        .unwrap_or_else(|| TrackedValue::public(serde_json::json!({})));
                    bindings.insert(name.clone(), value.clone())?;
                    scope.insert("vars".into(), bindings)?;
                    resolved.insert(name.clone(), value);
                }
                Err(failure) => {
                    unresolved.push((name, definition));
                    error = Some(failure);
                }
            }
        }
        if unresolved.len() == previous {
            return Err(error.unwrap_or_else(|| anyhow::anyhow!("cyclic task variable reference")));
        }
        pending = unresolved;
    }
    Ok((backend.evaluate_tracked(expression, &scope)?, resolved))
}

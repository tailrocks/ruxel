use super::{Execution, TrackedValue};
use crate::expression::{
    Expr, LookupResolver,
    provenance::{self, Provenance, VariableResolver},
};
use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::{cell::RefCell, collections::BTreeMap};

struct TaskVariables<'a, R: LookupResolver> {
    resolver: &'a R,
    definitions: &'a [Map<String, Value>],
    scope: &'a TrackedValue,
    resolved: RefCell<BTreeMap<(usize, String), TrackedValue>>,
    errors: RefCell<BTreeMap<(usize, String), anyhow::Error>>,
    active: RefCell<Vec<(usize, String)>>,
}

impl<R: LookupResolver> TaskVariables<'_, R> {
    fn get(&self, name: &str) -> Result<Option<TrackedValue>> {
        let ceiling = self
            .active
            .borrow()
            .last()
            .and_then(|(layer, active)| (active == name).then_some(*layer))
            .unwrap_or(self.definitions.len());
        let Some(index) = (0..ceiling)
            .rev()
            .find(|index| self.definitions[*index].contains_key(name))
        else {
            return Ok(None);
        };
        let binding = (index, name.to_owned());
        if let Some(value) = self.resolved.borrow().get(&binding) {
            return Ok(Some(value.clone()));
        }
        if let Some(error) = self.errors.borrow().get(&binding) {
            return Err(provenance::copy_evaluation_error(error));
        }
        if self.active.borrow().contains(&binding) {
            bail!("cyclic task variable reference");
        }
        let definition = &self.definitions[index][name];
        self.active.borrow_mut().push(binding.clone());
        let result = self.evaluate(definition);
        self.active.borrow_mut().pop();
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let copy = provenance::copy_evaluation_error(&error);
                self.errors.borrow_mut().insert(binding, error);
                return Err(copy);
            }
        };
        self.resolved.borrow_mut().insert(binding, value.clone());
        Ok(Some(value))
    }

    fn evaluate(&self, value: &Value) -> Result<TrackedValue> {
        if value
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind))
        {
            return provenance::evaluate_with_variables(
                &serde_json::from_value::<Expr>(value.clone())?,
                self.scope,
                self.resolver,
                self,
            );
        }
        match value {
            Value::Array(items) => {
                let values = items
                    .iter()
                    .map(|item| self.evaluate(item))
                    .collect::<Result<Vec<_>>>()?;
                Ok(TrackedValue {
                    value: Value::Array(values.iter().map(|item| item.value.clone()).collect()),
                    provenance: Provenance::Array(
                        values.into_iter().map(|item| item.provenance).collect(),
                    ),
                })
            }
            Value::Object(items) => Ok(TrackedValue::object(
                items
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), self.evaluate(value)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
            )),
            _ => Ok(TrackedValue::public(value.clone())),
        }
    }
}

impl<R: LookupResolver> VariableResolver for TaskVariables<'_, R> {
    fn resolve(&self, path: &[String], scope: &TrackedValue) -> Result<Option<TrackedValue>> {
        if !matches!(path.first().map(String::as_str), Some("vars")) {
            return Ok(None);
        }
        if path.len() == 1 {
            let mut vars = scope
                .get("vars")
                .unwrap_or_else(|| TrackedValue::public(serde_json::json!({})));
            for name in self.definitions.iter().flat_map(|layer| layer.keys()) {
                if let Some(value) = self.get(name)? {
                    vars.insert(name.clone(), value)?;
                }
            }
            return Ok(Some(vars));
        }
        let Some(value) = self.get(&path[1])? else {
            return Ok(None);
        };
        let mut temp = scope.clone();
        let mut vars = temp
            .get("vars")
            .unwrap_or_else(|| TrackedValue::public(serde_json::json!({})));
        vars.insert(path[1].clone(), value)?;
        temp.insert("vars".into(), vars)?;
        provenance::evaluate_tracked(
            &Expr::Ref {
                path: path.to_vec(),
            },
            &temp,
            self.resolver,
        )
        .map(Some)
    }
}

pub(super) fn evaluate(
    execution: &Execution,
    value: &Value,
    scope: &TrackedValue,
    definitions: &Map<String, Value>,
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    evaluate_layers(execution, value, scope, std::slice::from_ref(definitions))
}

pub(super) fn evaluate_layers(
    execution: &Execution,
    value: &Value,
    scope: &TrackedValue,
    definitions: &[Map<String, Value>],
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    evaluate_with_resolver(execution.lookups.as_ref(), value, scope, definitions)
}

pub(crate) fn evaluate_with_resolver<R: LookupResolver>(
    resolver: &R,
    value: &Value,
    scope: &TrackedValue,
    definitions: &[Map<String, Value>],
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    let variables = TaskVariables {
        resolver,
        definitions,
        scope,
        resolved: RefCell::new(BTreeMap::new()),
        errors: RefCell::new(BTreeMap::new()),
        active: RefCell::new(Vec::new()),
    };
    let result = variables.evaluate(value)?;
    let mut bindings = BTreeMap::new();
    for ((index, name), value) in variables.resolved.into_inner() {
        if definitions[index + 1..]
            .iter()
            .all(|layer| !layer.contains_key(&name))
        {
            bindings.insert(name, value);
        }
    }
    Ok((result, bindings))
}

#[cfg(test)]
mod tests;

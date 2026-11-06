use super::{Execution, TrackedValue};
use crate::expression::{
    Expr,
    provenance::{self, Provenance, VariableResolver},
};
use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::{cell::RefCell, collections::BTreeMap};

struct TaskVariables<'a> {
    execution: &'a Execution,
    definitions: &'a Map<String, Value>,
    scope: &'a TrackedValue,
    resolved: RefCell<BTreeMap<String, TrackedValue>>,
    errors: RefCell<BTreeMap<String, anyhow::Error>>,
    active: RefCell<Vec<String>>,
}

impl TaskVariables<'_> {
    fn get(&self, name: &str) -> Result<Option<TrackedValue>> {
        if let Some(value) = self.resolved.borrow().get(name) {
            return Ok(Some(value.clone()));
        }
        if let Some(error) = self.errors.borrow().get(name) {
            return Err(provenance::copy_evaluation_error(error));
        }
        let Some(definition) = self.definitions.get(name) else {
            return Ok(None);
        };
        if self.active.borrow().last().is_some_and(|key| key == name) {
            return Ok(None);
        }
        if self.active.borrow().iter().any(|key| key == name) {
            bail!("cyclic task variable reference");
        }
        self.active.borrow_mut().push(name.to_owned());
        let result = self.evaluate(definition);
        self.active.borrow_mut().pop();
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let copy = provenance::copy_evaluation_error(&error);
                self.errors.borrow_mut().insert(name.to_owned(), error);
                return Err(copy);
            }
        };
        self.resolved
            .borrow_mut()
            .insert(name.to_owned(), value.clone());
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
                self.execution.lookups.as_ref(),
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

impl VariableResolver for TaskVariables<'_> {
    fn resolve(&self, path: &[String], scope: &TrackedValue) -> Result<Option<TrackedValue>> {
        if !matches!(path.first().map(String::as_str), Some("vars" | "variables")) {
            return Ok(None);
        }
        if path.len() == 1 {
            let mut vars = scope
                .get("vars")
                .unwrap_or_else(|| TrackedValue::public(serde_json::json!({})));
            for name in self.definitions.keys() {
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
            self.execution.lookups.as_ref(),
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
    let variables = TaskVariables {
        execution,
        definitions,
        scope,
        resolved: RefCell::new(BTreeMap::new()),
        errors: RefCell::new(BTreeMap::new()),
        active: RefCell::new(Vec::new()),
    };
    let result = variables.evaluate(value)?;
    Ok((result, variables.resolved.into_inner()))
}

#[cfg(test)]
mod tests;

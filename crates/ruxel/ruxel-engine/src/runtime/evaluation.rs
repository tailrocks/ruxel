use super::{Execution, Lookups};
use crate::{
    expression::{
        self, Expr, Scope,
        provenance::{Provenance, TrackedValue},
    },
    secrets::Lookup,
};
use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

impl Execution {
    pub(super) fn prefetch_lookups(&self, value: &Value) -> Result<()> {
        fn direct_lookups(value: &Value, found: &mut Vec<Lookup>) -> Result<()> {
            if let Some(kind) = value.get("kind").and_then(Value::as_str) {
                if kind == "lookup" {
                    found.push(Lookups::lookup(&serde_json::from_value::<Expr>(
                        value.clone(),
                    )?)?);
                    return Ok(());
                }
                if ["literal", "ref", "call", "list", "map"].contains(&kind) {
                    return Ok(());
                }
            }
            match value {
                Value::Array(items) => {
                    for item in items {
                        direct_lookups(item, found)?;
                    }
                }
                Value::Object(items) => {
                    for item in items.values() {
                        direct_lookups(item, found)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut lookups = Vec::new();
        direct_lookups(value, &mut lookups)?;
        self.lookups
            .resolver
            .prefetch(&lookups)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub(super) fn evaluate_value_tracked(
        &self,
        value: &Value,
        scope: &TrackedValue,
    ) -> Result<TrackedValue> {
        if let Some(kind) = value.get("kind").and_then(Value::as_str)
            && ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind)
        {
            return expression::provenance::evaluate_tracked(
                &serde_json::from_value::<Expr>(value.clone())?,
                scope,
                self.lookups.as_ref(),
            );
        }
        match value {
            Value::Array(items) => {
                let items = items
                    .iter()
                    .map(|v| self.evaluate_value_tracked(v, scope))
                    .collect::<Result<Vec<_>>>()?;
                Ok(TrackedValue {
                    value: Value::Array(items.iter().map(|v| v.value.clone()).collect()),
                    provenance: Provenance::Array(
                        items.into_iter().map(|v| v.provenance).collect(),
                    ),
                })
            }
            Value::Object(items) => Ok(TrackedValue::object(
                items
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.evaluate_value_tracked(v, scope)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
            )),
            _ => Ok(TrackedValue::public(value.clone())),
        }
    }

    pub(super) fn evaluate_value(&self, value: &Value, scope: &Scope) -> Result<Value> {
        if let Some(kind) = value.get("kind").and_then(Value::as_str)
            && ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind)
        {
            return expression::evaluate(
                &serde_json::from_value::<Expr>(value.clone())?,
                scope,
                self.lookups.as_ref(),
            );
        }
        match value {
            Value::Array(items) => items
                .iter()
                .map(|v| self.evaluate_value(v, scope))
                .collect(),
            Value::Object(items) => items
                .iter()
                .map(|(k, v)| Ok((k.clone(), self.evaluate_value(v, scope)?)))
                .collect::<Result<Map<_, _>>>()
                .map(Value::Object),
            _ => Ok(value.clone()),
        }
    }
}

use super::{Backend, Map, Result, TrackedValue, Value, evaluate_standard_layered};
use crate::expression::{Expr, LookupResolver};
use std::{cell::RefCell, collections::BTreeMap};

struct BackendLookups<'a, B: ?Sized> {
    backend: RefCell<&'a mut B>,
    scope: &'a TrackedValue,
}

impl<B: Backend + ?Sized> LookupResolver for BackendLookups<'_, B> {
    fn resolve(&self, lookup: &Expr) -> Result<Value> {
        self.backend
            .borrow_mut()
            .evaluate_tracked(&serde_json::to_value(lookup)?, self.scope)
            .map(|value| value.value)
    }
}

pub(super) fn evaluate_layers<B: Backend + ?Sized>(
    backend: &mut B,
    expression: &Value,
    scope: &TrackedValue,
    layers: &[Map<String, Value>],
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    match layers {
        [] => Ok((
            backend.evaluate_tracked(expression, scope)?,
            BTreeMap::new(),
        )),
        [vars] => backend.evaluate_task_expression(expression, scope, vars),
        _ => evaluate_standard_layered(
            &BackendLookups {
                backend: RefCell::new(backend),
                scope,
            },
            expression,
            scope,
            layers,
        ),
    }
}

pub(super) fn evaluate<B: Backend + ?Sized>(
    backend: &mut B,
    expression: &Value,
    scope: &TrackedValue,
    vars: &Map<String, Value>,
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    evaluate_standard_layered(
        &BackendLookups {
            backend: RefCell::new(backend),
            scope,
        },
        expression,
        scope,
        std::slice::from_ref(vars),
    )
}

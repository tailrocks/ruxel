use super::{Map, Result, TrackedValue, Value};
use crate::expression::LookupResolver;
use std::collections::BTreeMap;

/// Evaluate standard expressions with demand-driven layered bindings.
///
/// A custom backend can delegate its layered evaluation to this function and
/// provide its own lookup resolver. Expressions use the engine's standard
/// semantics, rather than backend-specific expression extensions.
///
/// # Errors
/// Returns an error for an invalid expression, unresolved binding or lookup.
pub fn evaluate_standard_layered<R: LookupResolver>(
    resolver: &R,
    value: &Value,
    scope: &TrackedValue,
    layers: &[Map<String, Value>],
) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
    crate::runtime::lazy_vars::evaluate_with_resolver(resolver, value, scope, layers)
}

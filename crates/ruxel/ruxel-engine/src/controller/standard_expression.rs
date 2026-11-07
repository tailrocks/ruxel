use super::{Map, Result, TrackedValue, Value};
use crate::expression::LookupResolver;
use std::collections::BTreeMap;

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

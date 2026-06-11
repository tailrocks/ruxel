//! Controller-side task-result envelopes: the exact dict shapes Ansible
//! (core 2.21) gives registered variables for skip, loop aggregation,
//! until/retries, and changed_when — pinned by the runtime-semantics
//! goldens (tools/oracle/captures/runtime-semantics.jsonl) and reproduced
//! here field-for-field. Module execution fills the inner result dicts
//! (M3); these functions build everything around them.

use crate::playbook::Condition;
use minijinja::value::Value;

/// The result dict a task registers when its `when` evaluated false
/// (SEMANTICS §3.2). `false_condition` carries the failing condition as
/// written: an expression string, or the literal bool for `when: false`.
pub fn skipped_result(false_condition: &Condition) -> Value {
    let cond_value = match false_condition {
        Condition::Literal(b) => Value::from(*b),
        Condition::Expr(e) => Value::from(e.as_str()),
        Condition::All(exprs) => Value::from(exprs.first().map(String::as_str).unwrap_or("")),
    };
    Value::from_iter([
        ("changed", Value::from(false)),
        ("failed", Value::from(false)),
        ("false_condition", cond_value),
        ("skip_reason", Value::from("Conditional result was False")),
        ("skipped", Value::from(true)),
    ])
}

/// For a `when` list (AND), the registered `false_condition` is the first
/// entry that evaluated false — pick it given the per-entry outcomes.
pub fn first_false_condition(cond: &Condition, outcomes: &[bool]) -> Condition {
    match cond {
        Condition::All(exprs) => {
            let idx = outcomes.iter().position(|ok| !ok).unwrap_or(0);
            Condition::Expr(exprs.get(idx).cloned().unwrap_or_default())
        }
        other => other.clone(),
    }
}

/// Decorate one loop iteration's result with its item binding, as Ansible
/// records inside `results`: `ansible_loop_var: item` plus the item value.
pub fn decorate_loop_item(result: &Value, item: &Value) -> Value {
    merge(
        result,
        [
            ("ansible_loop_var", Value::from("item")),
            ("item", item.clone()),
        ],
    )
}

/// Aggregate a looped task's per-item results into the registered dict
/// (SEMANTICS §3.3), shapes pinned by goldens E3, E4, E10, E11:
/// - empty item list → `skipped: true`, `skip_reason`/`skipped_reason`
///   "No items in the list", empty `results`
/// - any failure → `failed: true`, msg "One or more items failed"
/// - all items skipped → msg "All items skipped" plus aggregate
///   `skipped: true`
/// - otherwise → msg "All items completed"
/// - `changed` is true iff any item changed.
pub fn loop_aggregate(results: Vec<Value>) -> Value {
    if results.is_empty() {
        return Value::from_iter([
            ("changed", Value::from(false)),
            ("failed", Value::from(false)),
            ("results", Value::from(Vec::<Value>::new())),
            ("skipped", Value::from(true)),
            ("skip_reason", Value::from("No items in the list")),
            ("skipped_reason", Value::from("No items in the list")),
        ]);
    }
    let truthy = |r: &Value, key: &str| {
        r.get_attr(key)
            .map(|v| !v.is_undefined() && v.is_true())
            .unwrap_or(false)
    };
    let changed = results.iter().any(|r| truthy(r, "changed"));
    let failed = results.iter().any(|r| truthy(r, "failed"));
    let all_skipped = results.iter().all(|r| truthy(r, "skipped"));
    let msg = if failed {
        "One or more items failed"
    } else if all_skipped {
        "All items skipped"
    } else {
        "All items completed"
    };
    let mut pairs = vec![
        ("changed", Value::from(changed)),
        ("failed", Value::from(failed)),
        ("msg", Value::from(msg)),
        ("results", Value::from(results)),
    ];
    if all_skipped {
        pairs.push(("skipped", Value::from(true)));
    }
    Value::from_iter(pairs)
}

/// Apply a `changed_when` outcome: the result's `changed` is replaced and
/// the raw outcome recorded as `changed_when_result` (golden E2).
pub fn apply_changed_when(result: &Value, outcome: bool) -> Value {
    merge(
        result,
        [
            ("changed", Value::from(outcome)),
            ("changed_when_result", Value::from(outcome)),
        ],
    )
}

/// Finalize an `until` task's registered result: the last attempt's dict
/// plus `attempts` (golden E5).
pub fn finalize_until(last: &Value, attempts: u64) -> Value {
    merge(last, [("attempts", Value::from(attempts))])
}

/// Shallow-merge extra keys into a map value (existing keys overwritten).
fn merge<const N: usize>(base: &Value, extra: [(&str, Value); N]) -> Value {
    let mut pairs: Vec<(Value, Value)> = Vec::new();
    if let Ok(iter) = base.try_iter() {
        for key in iter {
            if let Ok(v) = base.get_item(&key)
                && !extra.iter().any(|(k, _)| key.as_str() == Some(k))
            {
                pairs.push((key, v));
            }
        }
    }
    for (k, v) in extra {
        pairs.push((Value::from(k), v));
    }
    Value::from_iter(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_shape_for_literal_false() {
        let v = skipped_result(&Condition::Literal(false));
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            serde_json::json!({
                "changed": false, "failed": false, "false_condition": false,
                "skip_reason": "Conditional result was False", "skipped": true
            })
        );
    }

    #[test]
    fn merge_overwrites_existing_keys() {
        let base = Value::from_iter([("changed", Value::from(true)), ("rc", Value::from(0))]);
        let v = apply_changed_when(&base, false);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["changed"], false);
        assert_eq!(json["changed_when_result"], false);
        assert_eq!(json["rc"], 0);
    }

    #[test]
    fn empty_loop_shape() {
        let v = loop_aggregate(vec![]);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["skipped"], true);
        assert_eq!(json["skip_reason"], "No items in the list");
        assert_eq!(json["skipped_reason"], "No items in the list");
        assert_eq!(json["results"], serde_json::json!([]));
    }
}

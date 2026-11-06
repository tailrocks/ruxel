#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "legacy runtime behavior tests use direct fixture assertions"
)]

use minijinja::value::Value;
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver, Scope, VarValue};
use ruxel_core::playbook::Condition;
use ruxel_core::task_eval::{
    apply_changed_when, censored_result, decorate_loop_item, finalize_until, loop_aggregate,
    skipped_result,
};
use serde_json::Value as Json;
use std::sync::Arc;

fn as_json(value: &Value) -> Json {
    serde_json::to_value(value).expect("runtime value serializes to JSON")
}

fn engine() -> Engine {
    Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
}

#[test]
fn false_condition_produces_a_skipped_result() {
    assert_eq!(
        as_json(&skipped_result(&Condition::Literal(false))),
        serde_json::json!({
            "changed": false,
            "failed": false,
            "false_condition": false,
            "skip_reason": "Conditional result was False",
            "skipped": true
        })
    );
}

#[test]
fn changed_when_updates_changed_without_dropping_module_fields() {
    let base = Value::from_iter([
        ("changed", Value::from(false)),
        ("failed", Value::from(false)),
        ("rc", Value::from(0)),
    ]);
    let result = as_json(&apply_changed_when(&base, true));
    assert_eq!(result["changed"], true);
    assert_eq!(result["changed_when_result"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["rc"], 0);
}

#[test]
fn loop_items_keep_item_context_and_aggregate_status() {
    let first = decorate_loop_item(
        &Value::from_iter([("changed", Value::from(true))]),
        &Value::from("alpha"),
    );
    let second = decorate_loop_item(
        &skipped_result(&Condition::Expr("item != 'beta'".into())),
        &Value::from("beta"),
    );
    let first_json = as_json(&first);
    assert_eq!(first_json["item"], "alpha");
    assert_eq!(first_json["ansible_loop_var"], "item");
    let second_json = as_json(&second);
    assert_eq!(second_json["item"], "beta");
    assert_eq!(second_json["skipped"], true);

    let aggregate = as_json(&loop_aggregate(vec![first, second]));
    assert_eq!(aggregate["changed"], true);
    assert_eq!(aggregate["failed"], false);
    assert_eq!(aggregate["msg"], "All items completed");
    assert_eq!(aggregate["results"].as_array().unwrap().len(), 2);
}

#[test]
fn failed_loop_item_marks_the_aggregate_failed() {
    let aggregate = as_json(&loop_aggregate(vec![
        Value::from_iter([
            ("changed", Value::from(false)),
            ("failed", Value::from(false)),
        ]),
        Value::from_iter([
            ("changed", Value::from(true)),
            ("failed", Value::from(true)),
        ]),
    ]));
    assert_eq!(aggregate["changed"], true);
    assert_eq!(aggregate["failed"], true);
    assert_eq!(aggregate["msg"], "One or more items failed");
}

#[test]
fn all_skipped_loop_and_empty_loop_have_distinct_results() {
    let skipped = Value::from_iter([
        ("changed", Value::from(false)),
        ("skipped", Value::from(true)),
    ]);
    let aggregate = as_json(&loop_aggregate(vec![skipped.clone(), skipped]));
    assert_eq!(aggregate["msg"], "All items skipped");
    assert_eq!(aggregate["skipped"], true);

    let empty = as_json(&loop_aggregate(vec![]));
    assert_eq!(empty["results"], serde_json::json!([]));
    assert_eq!(empty["skipped"], true);
    assert_eq!(empty["skip_reason"], "No items in the list");
    assert_eq!(empty["skipped_reason"], "No items in the list");
}

#[test]
fn until_attempt_count_is_added_to_the_last_result() {
    let last = Value::from_iter([
        ("changed", Value::from(false)),
        ("failed", Value::from(false)),
        ("rc", Value::from(0)),
    ]);
    let result = as_json(&finalize_until(&last, 2));
    assert_eq!(result["attempts"], 2);
    assert_eq!(result["rc"], 0);
    assert_eq!(result["failed"], false);
}

#[test]
fn conditions_read_registered_values_and_render_typed_values() {
    let value = serde_json::json!({"rc": 0, "failed": false});
    let scope = Scope::new().with_layer(vec![(
        "result".to_string(),
        VarValue::Final(Value::from_serialize(&value)),
    )]);
    let engine = engine();
    assert!(
        engine
            .eval_condition(
                &Condition::Expr("result.rc == 0 and not result.failed".into()),
                &scope
            )
            .unwrap()
    );
    assert!(
        !engine
            .eval_condition(&Condition::Expr("result.rc != 0".into()), &scope)
            .unwrap()
    );
    assert_eq!(
        serde_json::to_value(engine.render_str("{{ result.failed }}", &scope).unwrap()).unwrap(),
        Json::Bool(false)
    );
}

#[test]
fn no_log_censors_output_and_preserves_changed_state_for_each_item() {
    let single = as_json(&censored_result(true, None));
    assert_eq!(single["changed"], true);
    assert_eq!(single["_ansible_no_log"], true);
    assert_eq!(
        single["censored"],
        "the output has been hidden due to the fact that 'no_log: true' was specified for this result"
    );
    assert!(single.get("stdout").is_none());

    let loop_result = as_json(&censored_result(true, Some(&[true, false])));
    assert_eq!(loop_result["changed"], true);
    assert_eq!(loop_result["results"][0]["changed"], true);
    assert_eq!(loop_result["results"][1]["changed"], false);
    assert!(loop_result["results"][0].get("stdout").is_none());
}

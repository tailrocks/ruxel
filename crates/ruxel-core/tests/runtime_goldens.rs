use minijinja::value::Value;
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver, Scope, VarValue};
use ruxel_core::playbook::Condition;
use ruxel_core::task_eval::{
    apply_changed_when, decorate_loop_item, finalize_until, loop_aggregate, skipped_result,
};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn load_dumps() -> HashMap<String, Json> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle/captures/runtime-semantics.jsonl");
    let content = std::fs::read_to_string(&path)
        .expect("runtime-semantics.jsonl — run tools/oracle/capture_runtime.sh first");
    let mut dumps = HashMap::new();
    for line in content.lines() {
        let rec: Json = serde_json::from_str(line).unwrap();
        let name = rec["task_name"].as_str().unwrap();
        if let Some(exp) = name.strip_suffix(" dump") {
            dumps.insert(exp.to_string(), rec["result"]["msg"].clone());
        }
    }
    assert!(dumps.len() >= 10, "expected all experiment dumps");
    dumps
}

fn to_value(j: &Json) -> Value {
    Value::from_serialize(j)
}

fn to_json(v: &Value) -> Json {
    serde_json::to_value(v).unwrap()
}

fn engine() -> Engine {
    Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
}

fn scope_with(pairs: &[(&str, &Json)]) -> Scope {
    Scope::new().with_layer(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), VarValue::Final(to_value(v))))
            .collect(),
    )
}

#[test]
fn e1_skip_shape_matches() {
    let dumps = load_dumps();
    let built = skipped_result(&Condition::Literal(false));
    assert_eq!(to_json(&built), dumps["E1"]);
}

#[test]
fn e2_changed_when_decoration_matches() {
    let dumps = load_dumps();
    let golden = &dumps["E2"];

    let mut base = golden.clone();
    base["changed"] = Json::Bool(true);
    base.as_object_mut().unwrap().remove("changed_when_result");

    let outcome = engine()
        .eval_condition(&Condition::Literal(false), &Scope::new())
        .unwrap();
    let built = apply_changed_when(&to_value(&base), outcome);
    assert_eq!(to_json(&built), *golden);
}

#[test]
fn e3_per_item_when_placement_and_aggregate_match() {
    let dumps = load_dumps();
    let golden = &dumps["E3"];
    let items = ["alpha", "beta", "gamma"];
    let eng = engine();

    let cond = Condition::Expr("item != 'beta'".into());
    let outcomes: Vec<bool> = items
        .iter()
        .map(|i| {
            let scope = scope_with(&[("item", &Json::String((*i).into()))]);
            eng.eval_condition(&cond, &scope).unwrap()
        })
        .collect();
    assert_eq!(outcomes, [true, false, true]);
    for (i, outcome) in outcomes.iter().enumerate() {
        let skipped = golden["results"][i]["skipped"].as_bool().unwrap_or(false);
        assert_eq!(skipped, !outcome, "skip placement at item {i}");
    }

    let built_skip = decorate_loop_item(&skipped_result(&cond), &Value::from("beta"));
    assert_eq!(to_json(&built_skip), golden["results"][1]);

    let results: Vec<Value> = golden["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(to_value)
        .collect();
    assert_eq!(to_json(&loop_aggregate(results)), *golden);
}

#[test]
fn e4_failed_aggregate_matches() {
    let dumps = load_dumps();
    let golden = &dumps["E4"];
    let results: Vec<Value> = golden["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(to_value)
        .collect();
    let built = loop_aggregate(results);
    assert_eq!(to_json(&built), *golden);
    assert_eq!(golden["msg"], "One or more items failed");
    assert_eq!(golden["failed"], true);
}

#[test]
fn e5_until_attempts_match() {
    let dumps = load_dumps();
    let golden = &dumps["E5"];
    assert_eq!(golden["attempts"], 2);

    let eng = engine();
    let cond = Condition::Expr("e5_until.rc == 0".into());
    let mut failing = golden.clone();
    failing["rc"] = Json::from(1);
    assert!(
        !eng.eval_condition(&cond, &scope_with(&[("e5_until", &failing)]))
            .unwrap()
    );
    assert!(
        eng.eval_condition(&cond, &scope_with(&[("e5_until", golden)]))
            .unwrap()
    );

    let mut last = golden.clone();
    last.as_object_mut().unwrap().remove("attempts");
    let built = finalize_until(&to_value(&last), 2);
    assert_eq!(to_json(&built), *golden);
}

#[test]
fn e6_e7_registered_failure_feeds_conditions() {
    let dumps = load_dumps();
    let e6 = &dumps["E6"];
    assert_eq!(e6["failed"], true);

    let eng = engine();
    let e2 = &dumps["E2"];
    let scope = scope_with(&[("e2_cmd", e2), ("e6_fail", e6)]);
    let cond = Condition::All(vec!["e2_cmd.rc == 0".into(), "e6_fail.failed".into()]);
    assert!(eng.eval_condition(&cond, &scope).unwrap());
    assert_eq!(dumps["E7"]["failed"], false, "E7 executed (not skipped)");
}

#[test]
fn e8_set_fact_expression_matches() {
    let dumps = load_dumps();
    let eng = engine();
    let scope = scope_with(&[("e2_cmd", &dumps["E2"])]);
    let v = eng.render_str("{{ e2_cmd.rc == 0 }}", &scope).unwrap();
    assert_eq!(to_json(&v), dumps["E8"]);
}

#[test]
fn e9_concat_stringifies_python_style() {
    let dumps = load_dumps();

    let eng = engine();
    let item = serde_json::json!({"item": "one", "stat": {"exists": false}});
    let scope = scope_with(&[("item", &item)]);
    let v = eng
        .render_str("{{ item.item }}={{ item.stat.exists }}", &scope)
        .unwrap();
    assert_eq!(v.as_str(), Some("one=False"));
    let _ = dumps;
}

#[test]
fn e10_all_skipped_aggregate_matches() {
    let dumps = load_dumps();
    let golden = &dumps["E10"];
    let results: Vec<Value> = golden["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(to_value)
        .collect();
    assert_eq!(to_json(&loop_aggregate(results)), *golden);
    assert_eq!(golden["msg"], "All items skipped");
}

#[test]
fn e11_empty_loop_aggregate_matches() {
    let dumps = load_dumps();
    assert_eq!(to_json(&loop_aggregate(vec![])), dumps["E11"]);
}

#[test]
fn e12_e13_no_log_censors_output_not_register() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle/captures/runtime-semantics.jsonl");
    let content = std::fs::read_to_string(&path).unwrap();
    let mut e12_output = None;
    let mut e13_output = None;
    for line in content.lines() {
        let rec: Json = serde_json::from_str(line).unwrap();
        match (
            rec["task_name"].as_str().unwrap(),
            rec["status"].as_str().unwrap(),
        ) {
            ("E12 no_log result is censored in output", "ok") => {
                e12_output = Some(rec["result"].clone());
            }
            ("E13 no_log loop censors per item", "ok") => {
                e13_output = Some(rec["result"].clone());
            }
            _ => {}
        }
    }
    let e12_output = e12_output.expect("E12 capture record");
    let e13_output = e13_output.expect("E13 capture record");

    assert_eq!(
        to_json(&ruxel_core::task_eval::censored_result(true, None)),
        e12_output
    );
    assert_eq!(
        to_json(&ruxel_core::task_eval::censored_result(
            true,
            Some(&[true, true])
        )),
        e13_output
    );

    let dumps = load_dumps();
    assert_eq!(dumps["E12"]["stdout"], "topsecret");
    assert_eq!(dumps["E13"]["results"][0]["stdout"], "s1");
}

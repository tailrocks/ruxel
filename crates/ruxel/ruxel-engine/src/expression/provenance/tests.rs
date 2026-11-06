use super::*;
use serde_json::json;

struct Synthetic;
impl LookupResolver for Synthetic {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        Ok(json!("sample / private"))
    }
}
fn eval(expr: Value, scope: &TrackedValue) -> Result<TrackedValue> {
    evaluate_tracked(&serde_json::from_value(expr)?, scope, &Synthetic)
}
fn empty_scope() -> TrackedValue {
    TrackedValue::public(json!({}))
}
fn lookup() -> Value {
    json!({"kind":"lookup", "provider":"synthetic", "key":"unused"})
}
fn call(function: &str, args: Vec<Value>) -> Value {
    json!({"kind":"call", "function":function, "args":args})
}

#[test]
fn transforms_and_nested_values_preserve_secret_provenance() -> Result<()> {
    let expr = json!({"kind":"map", "entries":{
        "encoded":call("base64Encode", vec![lookup()]),
        "public":{"kind":"literal", "value":"visible"},
        "nested":call("urlencode", vec![call("base64Encode", vec![lookup()])])
    }});
    let value = eval(expr, &empty_scope())?;
    assert_eq!(value.value["encoded"], "c2FtcGxlIC8gcHJpdmF0ZQ==");
    assert_eq!(
        value.redacted(),
        json!({"encoded":"[REDACTED]", "public":"visible", "nested":"[REDACTED]"})
    );
    Ok(())
}

#[test]
fn references_select_provenance_without_tainting_public_siblings() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"vars":{"a":"same", "b":"same"}}),
        provenance: Provenance::Object(BTreeMap::from([(
            "vars".into(),
            Provenance::Object(BTreeMap::from([("a".into(), Provenance::Secret)])),
        )])),
    };
    let path = |key| json!({"kind":"ref", "path":["vars", key]});
    assert!(eval(path("a"), &scope)?.is_secret());
    assert!(!eval(path("b"), &scope)?.is_secret());
    assert_eq!(
        eval(call("base64Encode", vec![path("a")]), &scope)?.redacted(),
        json!("[REDACTED]")
    );
    Ok(())
}

#[test]
fn lazy_condition_and_map_propagate_sensitivity() -> Result<()> {
    let sensitive_test = call("eq", vec![lookup(), lookup()]);
    let value = eval(
        call(
            "if",
            vec![
                sensitive_test,
                json!({"kind":"literal", "value":"selected"}),
                json!({"kind":"ref", "path":["missing"]}),
            ],
        ),
        &empty_scope(),
    )?;
    assert!(value.is_secret());
    let list = json!({"kind":"list", "items":[lookup(), {"kind":"literal", "value":"public"}]});
    let mapped = eval(
        call(
            "map",
            vec![
                list,
                call("base64Encode", vec![json!({"kind":"ref", "path":["item"]})]),
            ],
        ),
        &empty_scope(),
    )?;
    assert_eq!(mapped.redacted(), json!(["[REDACTED]", "cHVibGlj"]));
    Ok(())
}

#[test]
fn sensitive_failures_are_generic_and_public_errors_remain_useful() {
    let invalid = call("base64Decode", vec![lookup()]);
    let error = eval(invalid, &empty_scope())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert_eq!(error, "sensitive expression failed");
    let error = eval(
        call("base64Decode", vec![json!({"kind":"literal", "value":"%"})]),
        &empty_scope(),
    )
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
    assert_ne!(error, "sensitive expression failed");
    assert!(!error.is_empty());
}

#[test]
fn missing_secret_reference_taints_default_and_short_array_tree_fails_closed() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"vars":{"secret":{}}, "item":["synthetic-secret"]}),
        provenance: Provenance::Object(BTreeMap::from([
            (
                "vars".into(),
                Provenance::Object(BTreeMap::from([("secret".into(), Provenance::Secret)])),
            ),
            ("item".into(), Provenance::Array(vec![])),
        ])),
    };
    let missing = json!({"kind":"ref", "path":["vars", "secret", "probe"]});
    let fallback = json!({"kind":"literal", "value":"absent"});
    assert!(eval(call("default", vec![missing, fallback]), &scope)?.is_secret());
    assert!(eval(json!({"kind":"ref", "path":["item", "0"]}), &scope)?.is_secret());
    Ok(())
}

#[test]
fn wrong_provenance_shape_fails_closed_for_references_and_get() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"item":["synthetic-secret"]}),
        provenance: Provenance::Object(BTreeMap::from([(
            "item".into(),
            Provenance::Object(BTreeMap::new()),
        )])),
    };
    assert!(scope.is_secret());
    assert!(
        scope
            .get("item")
            .and_then(|v| v.get("0"))
            .context("item")?
            .is_secret()
    );
    assert!(eval(json!({"kind":"ref", "path":["item", "0"]}), &scope)?.is_secret());
    Ok(())
}

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
fn call(function: &str, args: &[Value]) -> Value {
    json!({"kind":"call", "function":function, "args":args})
}

#[test]
fn transforms_and_nested_values_preserve_secret_provenance() -> Result<()> {
    let expr = json!({"kind":"map", "entries":{
        "encoded":call("base64Encode", &[lookup()]),
        "public":{"kind":"literal", "value":"visible"},
        "nested":call("urlencode", &[call("base64Encode", &[lookup()])])
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
        eval(call("base64Encode", &[path("a")]), &scope)?.redacted(),
        json!("[REDACTED]")
    );
    Ok(())
}

#[test]
fn lazy_condition_and_map_propagate_sensitivity() -> Result<()> {
    let sensitive_test = call("eq", &[lookup(), lookup()]);
    let value = eval(
        call(
            "if",
            &[
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
            &[
                list,
                call("base64Encode", &[json!({"kind":"ref", "path":["item"]})]),
            ],
        ),
        &empty_scope(),
    )?;
    assert_eq!(mapped.redacted(), json!(["[REDACTED]", "cHVibGlj"]));
    Ok(())
}

#[test]
fn sensitive_failures_are_generic_and_public_errors_remain_useful() {
    let invalid = call("base64Decode", &[lookup()]);
    let error = eval(invalid, &empty_scope())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert_eq!(error, "sensitive expression failed");
    let error = eval(
        call("base64Decode", &[json!({"kind":"literal", "value":"%"})]),
        &empty_scope(),
    )
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
    assert_ne!(error, "sensitive expression failed");
    assert_ne!(error, "");
}

mod review_tests;

#[test]
fn flat_map_preserves_nested_secret_provenance() -> Result<()> {
    let source = json!({"kind":"list", "items":[lookup(), {"kind":"literal", "value":"visible"}]});
    let body = json!({"kind":"list", "items":[{"kind":"ref", "path":["entry"]}]});
    let result = eval(
        call(
            "flatMap",
            &[source, json!({"kind":"literal", "value":"entry"}), body],
        ),
        &empty_scope(),
    )?;
    assert_eq!(result.redacted(), json!(["[REDACTED]", "visible"]));
    Ok(())
}

use super::*;

fn key(host: &str, value: &str, params: &Value) -> Result<String, LedgerKeyError> {
    let secret = Secret::new("K8S_TOKEN", value.to_owned());
    ledger_key("fleet", host, "resource", "file", params, &|name| {
        (name == secret.name()).then_some(&secret)
    })
}

#[test]
fn canonical_order_host_and_secret_rotation_affect_key() {
    let a = serde_json::json!({"z": 1, "a": 2});
    let b: Value = serde_json::from_str(r#"{"a":2,"z":1}"#).expect("valid JSON");
    assert_eq!(key("host", "one", &a), key("host", "one", &b));
    assert_ne!(key("host", "one", &a), key("other", "one", &a));
    let secret_ref = serde_json::json!({"token":{"kind":"secretRef","name":"K8S_TOKEN"}});
    let first = key("host", "one", &secret_ref).expect("secret exists");
    let rotated = key("host", "two", &secret_ref).expect("secret exists");
    assert_ne!(first, rotated);
    assert!(!first.contains("one"));
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn missing_secret_is_named_and_values_never_leak() {
    let params = serde_json::json!({"kind":"secretRef","name":"MISSING"});
    let result = ledger_key("f", "h", "id", "type", &params, &|_| None);
    assert_eq!(result, Err(LedgerKeyError::MissingSecret("MISSING".into())));
    let secret_params = serde_json::json!({"kind":"secretRef","name":"K8S_TOKEN"});
    let secret = Secret::new("K8S_TOKEN", "super-secret-value".into());
    let result =
        ledger_key("f", "h", "id", "type", &secret_params, &|_| Some(&secret)).expect("key");
    assert!(!result.contains(secret.expose()));
}

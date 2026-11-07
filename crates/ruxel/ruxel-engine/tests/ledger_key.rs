use ruxel_engine::{ledger_key::ledger_key, secret::Secret};

#[test]
fn integration_key_is_stable_and_secret_safe() {
    let value = Secret::new("TOKEN", "fixture-secret-value".into());
    let params =
        serde_json::json!({"credentials":{"kind":"secretRef","name":"TOKEN"},"mode":"0600"});
    let key = ledger_key("fleet", "host", "id", "file", &params, &|name| {
        (name == "TOKEN").then_some(&value)
    })
    .expect("secret exists");
    assert_eq!(key.len(), 64);
    assert!(!key.contains(value.expose()));
}

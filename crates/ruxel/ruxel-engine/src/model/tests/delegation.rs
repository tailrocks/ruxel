use super::*;

#[test]
fn delegation_authorization_is_tied_to_the_same_task_target() {
    for (key, val) in [
        ("delegateAllow", json!(["node"])),
        ("delegateLocalhost", json!(false)),
    ] {
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0][key] = val;
        assert!(validate_config(&value).is_err(), "{key}");
    }
    let mut value = config(json!({"kind":"debug"}));
    value["plays"][0]["tasks"][0]["delegateTo"] = json!("node");
    assert!(validate_config(&value).is_err());
    value["plays"][0]["tasks"][0]["delegateAllow"] = json!(["node"]);
    assert!(validate_config(&value).is_ok());
}

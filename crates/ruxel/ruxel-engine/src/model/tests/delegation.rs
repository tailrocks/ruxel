use super::*;

#[test]
fn private_copy_without_delegation_remains_valid() {
    let mut value = config(json!({"kind":"copy", "dest":"/tmp/payload", "content":"fixture"}));
    value["plays"][0]["tasks"][0]["noLog"] = json!(true);
    let result = validate_config(&value);
    assert!(result.is_ok(), "{result:?}");
}

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
    value["plays"][0]["tasks"][0]["delegateTo"] = json!("synthetic");
    assert!(validate_config(&value).is_err());
    value["plays"][0]["tasks"][0]["delegateAllow"] = json!(["synthetic"]);
    assert!(validate_config(&value).is_ok());
}

#[test]
fn literal_delegation_must_exist_in_inventory_at_any_depth() {
    let mut value = config(json!({"kind":"debug"}));
    for destination in ["synthetic", "localhost"] {
        value["plays"][0]["tasks"][0]["delegateTo"] = json!(destination);
        value["plays"][0]["tasks"][0]["delegateAllow"] = json!([destination]);
        assert!(validate_config(&value).is_ok());
    }
    value["plays"][0]["tasks"][0]["delegateTo"] = json!("missing");
    value["plays"][0]["tasks"][0]["delegateAllow"] = json!(["missing"]);
    assert!(validate_config(&value).is_err());
    value["plays"][0]["tasks"][0]["delegateTo"] = json!({"kind":"ref", "path":["vars", "host"]});
    assert!(validate_config(&value).is_ok());
    value["plays"][0]["tasks"][0] = json!({"name":"block", "block":[{"name":"nested", "delegateTo":"missing", "delegateAllow":["missing"], "operation":{"kind":"debug"}}]});
    assert!(validate_config(&value).is_err());
}

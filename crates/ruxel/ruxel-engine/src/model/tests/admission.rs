use super::*;

#[test]
fn validation_preserves_inventory_declaration_order() -> Result<()> {
    let value = json!({
        "inventory": {"hosts": {
            "zeta": {"address": "zeta.invalid"},
            "alpha": {"address": "alpha.invalid"}
        }},
        "plays": []
    });
    let validated = validate_config(&value)?;
    let hosts = validated["inventory"]["hosts"]
        .as_object()
        .context("hosts")?;
    assert_eq!(
        hosts.keys().map(String::as_str).collect::<Vec<_>>(),
        ["zeta", "alpha"]
    );
    Ok(())
}

#[test]
fn admission_configuration_keeps_full_input_and_inventory_order() -> Result<()> {
    let input = json!({"config": {
        "inventory": {"hosts": {"zeta": {"address": "zeta.invalid"}, "alpha": {"address": "alpha.invalid"}}},
        "plays": [],
        "ssh": {"identityFile": "keys/id", "knownHostsFile": "keys/hosts"}
    }});
    let guard = json!({"allow": ["192.0.2.0/24"], "deny": []});
    let loaded = AdmissionConfig::new(&input, &guard)?;
    assert_eq!(loaded.evaluated(), &input);
    assert_eq!(loaded.ssh().user(), "root");
    assert_eq!(loaded.guard().allow(), ["192.0.2.0/24"]);
    let hosts = loaded.normalized()["inventory"]["hosts"]
        .as_object()
        .context("hosts")?;
    assert_eq!(
        hosts.keys().map(String::as_str).collect::<Vec<_>>(),
        ["zeta", "alpha"]
    );
    assert!(AdmissionConfig::new(&input, &json!({"allow":["invalid"],"deny":[]})).is_err());
    let mut missing = input;
    missing["config"]
        .as_object_mut()
        .context("config")?
        .remove("ssh");
    assert!(AdmissionConfig::new(&missing, &guard).is_err());
    Ok(())
}

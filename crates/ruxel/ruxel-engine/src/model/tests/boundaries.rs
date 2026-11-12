use super::*;

#[test]
fn lookup_accepts_command_argv_and_rejects_legacy_pipe() {
    let mut value = config(json!({"kind":"debug", "msg":{
        "kind":"lookup", "provider":"command", "argv":["printf", "synthetic"]
    }}));
    assert!(validate_config(&value).is_ok());
    value["plays"][0]["tasks"][0]["operation"]["msg"] = json!({
        "kind":"lookup", "provider":"pipe", "key":"printf synthetic"
    });
    assert!(validate_config(&value).is_err());
}

#[test]
fn fnox_keys_follow_the_store_grammar_before_execution() -> Result<()> {
    let mut value = config(json!({"kind":"debug"}));
    value["plays"][0]["tasks"]
        .as_array_mut()
        .context("tasks")?
        .push(json!({
            "name":"later lookup", "operation":{"kind":"debug", "msg":{
                "kind":"lookup", "provider":"fnox", "key":"KEY"
            }}
        }));
    for key in [
        "",
        "1KEY",
        "KEY-NAME",
        "KEY.NAME",
        "KEY NAME",
        "KEY\n",
        "éKEY",
        "KEYé",
        "a",
        "_",
        "_KEY_123",
        "key123",
        "app_secret",
        "K".repeat(129).as_str(),
    ] {
        value["plays"][0]["tasks"][1]["operation"]["msg"]["key"] = json!(key);
        assert!(validate_config(&value).is_err(), "invalid fnox key {key:?}");
    }
    value["plays"][0]["tasks"][1]["operation"]["msg"]["key"] = Value::Null;
    assert!(validate_config(&value).is_err());
    value["plays"][0]["tasks"][1]["operation"]["msg"]
        .as_object_mut()
        .context("lookup")?
        .remove("key");
    assert!(validate_config(&value).is_err());
    for key in ["KEY", "KEY_1", "A1_B2", "K".repeat(128).as_str()] {
        value["plays"][0]["tasks"][1]["operation"]["msg"]["key"] = json!(key);
        validate_config(&value)?;
    }
    value["plays"][0]["tasks"][1]["operation"]["msg"] = json!({
        "kind":"lookup", "provider":"synthetic", "key":"fixture-key"
    });
    validate_config(&value)?;
    Ok(())
}

#[test]
fn retries_are_optional_nonnegative_and_cannot_overflow() -> Result<()> {
    let mut value = config(json!({"kind":"debug"}));
    assert_eq!(
        validate_config(&value)?["plays"][0]["tasks"][0]["retries"],
        Value::Null
    );
    for retries in [0, 1, 2] {
        value["plays"][0]["tasks"][0]["retries"] = json!(retries);
        assert_eq!(
            validate_config(&value)?["plays"][0]["tasks"][0]["retries"],
            retries
        );
    }
    value["plays"][0]["tasks"][0]["retries"] = json!(-1);
    assert!(validate_config(&value).is_err());
    value["plays"][0]["tasks"][0]["retries"] = json!(u64::MAX);
    assert!(validate_config(&value).is_err());
    Ok(())
}

#[test]
fn retry_delay_defaults_to_five_without_overriding_explicit_zero() -> Result<()> {
    let mut value = config(json!({"kind":"debug"}));
    assert_eq!(
        validate_config(&value)?["plays"][0]["tasks"][0]["delaySeconds"],
        5
    );
    for delay in [0, 2] {
        value["plays"][0]["tasks"][0]["delaySeconds"] = json!(delay);
        assert_eq!(
            validate_config(&value)?["plays"][0]["tasks"][0]["delaySeconds"],
            delay
        );
    }
    Ok(())
}

#[test]
fn rescue_and_always_require_block_tasks() {
    for section in ["rescue", "always"] {
        let child = json!({"name":"cleanup","operation":{"kind":"debug"}});
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0][section] = json!([child.clone()]);
        assert!(validate_config(&value).is_err(), "operation with {section}");
        value["plays"][0]["tasks"][0]["operation"] = Value::Null;
        value["plays"][0]["tasks"][0]["block"] = json!([child]);
        assert!(validate_config(&value).is_ok(), "block with {section}");
    }
}

#[test]
fn loop_variables_cannot_shadow_scope_roots() {
    for name in ["vars", "facts", "results", "inventoryHost", "setFacts"] {
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0]["loopVar"] = json!(name);
        assert!(
            validate_config(&value).is_err(),
            "reserved loop variable {name}"
        );
    }
    for name in ["item", "entry", "variables"] {
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0]["loopVar"] = json!(name);
        assert!(
            validate_config(&value).is_ok(),
            "valid loop variable {name}"
        );
    }
}

#[test]
fn apt_repository_requires_filename_before_execution() -> Result<()> {
    assert!(
        validate_config(&config(
            json!({"kind":"apt_repository","repo":"deb fixture"})
        ))
        .is_err()
    );
    assert!(
        validate_config(&config(
            json!({"kind":"apt_repository","repo":"deb fixture","filename":"fixture"})
        ))
        .is_ok()
    );
    assert!(validate_config(&config(json!({"kind":"apt_repository","repo":"deb fixture","filename":{"kind":"ref","path":["vars","filename"]}}))).is_ok());
    let params = json!({"repo":"deb fixture"});
    assert!(validate_operation("apt_repository", params.as_object().context("object")?).is_err());
    Ok(())
}

#[test]
fn set_fact_requires_values_mapping() {
    assert!(validate_config(&config(json!({"kind":"set_fact"}))).is_err());
    assert!(validate_config(&config(json!({"kind":"set_fact", "values":null}))).is_err());
    assert!(validate_config(&config(json!({"kind":"set_fact", "values":{}}))).is_ok());
    assert!(
        validate_config(&config(
            json!({"kind":"set_fact", "values":{"answer":{"kind":"literal","value":42}}})
        ))
        .is_ok()
    );
}

#[test]
fn blocks_reject_loop_controls() {
    let child = json!({"name":"child", "operation":{"kind":"debug"}});
    for (field, control) in [
        ("loop", json!({"kind":"list", "items":[]})),
        ("loopVar", json!("entry")),
        ("loopLabel", json!({"kind":"literal", "value":"label"})),
    ] {
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0]["operation"] = Value::Null;
        value["plays"][0]["tasks"][0]["block"] = json!([child.clone()]);
        value["plays"][0]["tasks"][0][field] = control.clone();
        assert!(validate_config(&value).is_err(), "block with {field}");
        value["plays"][0]["tasks"][0]["operation"] = json!({"kind":"debug"});
        value["plays"][0]["tasks"][0]["block"] = json!([]);
        assert!(validate_config(&value).is_ok(), "operation with {field}");
    }
}

/// A hold is part of an apt operation: the engine admits it, and only a
/// flag is a hold (T9).
#[test]
fn apt_operation_accepts_a_hold() -> Result<()> {
    let params = json!({"name": "pkg", "hold": true});
    validate_operation("apt", params.as_object().context("object")?)?;
    let listed = json!({"name": "pkg", "hold": ["pkg"]});
    assert!(
        validate_operation("apt", listed.as_object().context("object")?).is_err(),
        "a hold is a flag, not a list"
    );
    Ok(())
}

/// A key and a firewall rule can be removed, so the engine admits the
/// "absent" state of both.
#[test]
fn authorized_key_state_absent_and_firewall_state_are_valid() -> Result<()> {
    let key = json!({"key":"ssh-ed25519 AAAAC3Nza", "user":"deploy", "state":"absent"});
    validate_operation("authorized_key", key.as_object().context("object")?)?;
    let firewall = json!({"chain":"INPUT", "state":"absent"});
    validate_operation("iptables", firewall.as_object().context("object")?)?;
    Ok(())
}

use super::*;
use serde_json::json;
fn config(operation: Value) -> Value {
    let mut config = json!({"inventory":{"hosts":{"synthetic":{"address":"unused.invalid"}}},"plays":[{"name":"test","targets":["synthetic"],"tasks":[{"name":"test"}]}]});
    config["plays"][0]["tasks"][0]["operation"] = operation;
    config
}

#[test]
fn rejects_untyped_unknown_fields_and_operation_variants() {
    assert!(validate_config(&json!({"anything":true})).is_err());
    assert!(validate_config(&config(json!({"kind":"unknown"}))).is_err());
    assert!(
        validate_config(&config(
            json!({"kind":"copy","dest":"/tmp/synthetic","content":"ok","typo":true})
        ))
        .is_err()
    );
}
#[test]
fn rejects_invalid_evaluated_expression_results() {
    assert!(
        validate_operation(
            "copy",
            json!({"dest":"/tmp/synthetic","content":"ok","force":"wrong"})
                .as_object()
                .context("object")
                .unwrap_or(&Map::new())
        )
        .is_err()
    );
    assert!(
        validate_operation(
            "filesystem",
            json!({"dev":"/tmp/synthetic","fstype":"btrfs"})
                .as_object()
                .context("object")
                .unwrap_or(&Map::new())
        )
        .is_err()
    );
}
#[test]
fn checks_nested_expression_shape_before_execution() {
    let value = config(
        json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"call","function":"unknown","args":[]}}),
    );
    assert!(validate_config(&value).is_err());
    let value = config(
        json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"ref","path":[],"typo":true}}),
    );
    assert!(validate_config(&value).is_err());
}
#[test]
fn accepts_typed_deferred_values_and_plain_valid_results() -> Result<()> {
    validate_config(&config(
        json!({"kind":"copy","dest":"/tmp/synthetic","content":{"kind":"literal","value":"ok"}}),
    ))?;
    validate_operation(
        "copy",
        json!({"dest":"/tmp/synthetic","content":"ok","force":true})
            .as_object()
            .context("object")?,
    )?;
    Ok(())
}

#[test]
fn rejects_ssh_option_injection_before_transport() {
    let mut value = config(json!({"kind":"command","cmd":"safe"}));
    value["inventory"]["hosts"]["synthetic"]["address"] = json!("-oProxyCommand=unsafe");
    assert!(validate_config(&value).is_err());
    value["inventory"]["hosts"]["synthetic"]["address"] = json!("unused.invalid");
    value["inventory"]["hosts"]["synthetic"]["user"] = json!("bad user");
    assert!(validate_config(&value).is_err());
}

#[test]
fn unsupported_surface_fields_are_rejected_before_execution() -> anyhow::Result<()> {
    for (kind, value) in [
        ("command", json!({"cmd":"fixture", "removes":"fixture"})),
        ("shell", json!({"cmd":"fixture", "removes":"fixture"})),
        (
            "community.general.timezone",
            json!({"name":"UTC", "state":"present"}),
        ),
        (
            "community.postgresql.postgresql_db",
            json!({"name":"fixture", "login_db":"fixture"}),
        ),
        (
            "community.postgresql.postgresql_user",
            json!({"name":"fixture", "login_db":"fixture"}),
        ),
    ] {
        let params = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("fixture operation params"))?;
        assert!(validate_operation(kind, params).is_err(), "{kind}");
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
    for name in [
        "vars",
        "variables",
        "facts",
        "results",
        "inventoryHost",
        "setFacts",
    ] {
        let mut value = config(json!({"kind":"debug"}));
        value["plays"][0]["tasks"][0]["loopVar"] = json!(name);
        assert!(
            validate_config(&value).is_err(),
            "reserved loop variable {name}"
        );
    }
    for name in ["item", "entry"] {
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

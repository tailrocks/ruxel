use super::*;

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

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

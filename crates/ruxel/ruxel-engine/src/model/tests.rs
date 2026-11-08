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
    ] {
        let params = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("fixture operation params"))?;
        assert!(validate_operation(kind, params).is_err(), "{kind}");
    }
    Ok(())
}

mod boundaries;

#[test]
fn postgres_database_and_role_accept_login_db() -> anyhow::Result<()> {
    for kind in [
        "community.postgresql.postgresql_db",
        "community.postgresql.postgresql_user",
    ] {
        let params = json!({"name":"fixture", "login_db":"maintenance"});
        assert!(
            validate_operation(
                kind,
                params
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("expected params object"))?
            )
            .is_ok(),
            "{kind}"
        );
    }
    Ok(())
}

#[test]
fn onepassword_lookup_accepts_item_defaults_and_rejects_empty_selectors() -> Result<()> {
    for lookup in [
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item"}),
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "vault":null, "field":null}),
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "field":"username"}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password"}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password", "item":null}),
    ] {
        validate_config(&config(json!({"kind":"debug", "msg":lookup})))?;
    }
    for lookup in [
        json!({"kind":"lookup", "provider":"op"}),
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "reference":""}),
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "vault":""}),
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "field":""}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password", "item":"other"}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password", "vault":"other"}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password", "section":"other"}),
        json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/password", "field":"other"}),
    ] {
        assert!(validate_config(&config(json!({"kind":"debug", "msg":lookup}))).is_err());
    }
    Ok(())
}

mod block_controls;
mod delegation;
mod file_paths;

mod notifications;

use super::*;

#[test]
fn sysctl_key_is_checked_before_and_after_expression_evaluation() {
    for key in ["", "-bad", "net.key=bad", "net key", "net\nkey"] {
        let op = json!({"kind":"sysctl", "name":key, "value":"1"});
        assert!(validate_config(&config(op.clone())).is_err(), "{key:?}");
        assert!(
            validate_operation(
                "sysctl",
                json!({"name":key, "value":"1"})
                    .as_object()
                    .unwrap_or(&Map::new())
            )
            .is_err()
        );
    }
    for key in ["net.ipv4.ip_forward", "net/ipv4/conf/eth0.100/forwarding"] {
        assert!(
            validate_config(&config(json!({"kind":"sysctl", "name":key, "value":"1"}))).is_ok()
        );
    }
    assert!(
        validate_config(&config(
            json!({"kind":"sysctl", "name":{"kind":"ref", "path":["vars", "key"]}, "value":"1"})
        ))
        .is_ok()
    );
}

#[test]
fn url_and_timezone_validate_literals_and_evaluated_results() -> Result<()> {
    for (kind, field, valid, invalid) in [
        (
            "get_url",
            "url",
            vec!["http://example.invalid/x", "https://example.invalid/x"],
            vec!["", "file:///tmp/x", "HTTPS://example.invalid/x"],
        ),
        (
            "timezone",
            "name",
            vec!["UTC", "Asia/Tokyo"],
            vec!["", "--help"],
        ),
    ] {
        for (values, expected) in [(valid, true), (invalid, false)] {
            for value in values {
                let mut params = Map::new();
                params.insert(field.into(), json!(value));
                if kind == "get_url" {
                    params.insert("dest".into(), json!("/tmp/x"));
                }
                assert_eq!(
                    validate_operation(kind, &params).is_ok(),
                    expected,
                    "{kind}: {value:?}"
                );
                let mut op = params.clone();
                op.insert("kind".into(), json!(kind));
                assert_eq!(
                    validate_config(&config(Value::Object(op))).is_ok(),
                    expected,
                    "{kind}: {value:?}"
                );
            }
        }
        let mut op = Map::new();
        op.insert("kind".into(), json!(kind));
        op.insert(
            field.into(),
            json!({"kind":"ref", "path":["vars", "value"]}),
        );
        if kind == "get_url" {
            op.insert("dest".into(), json!("/tmp/x"));
        }
        validate_config(&config(Value::Object(op)))?;
    }
    Ok(())
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

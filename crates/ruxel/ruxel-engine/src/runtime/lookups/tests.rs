use super::*;
use crate::{
    expression::{Scope, evaluate},
    secrets::Backend,
};
use serde_json::json;
use std::sync::Arc;
struct Fake;
impl Backend for Fake {
    fn onepassword_reference(&self, _: &str) -> Result<String, String> {
        Err("unexpected".into())
    }
    fn fnox(&self, _: &str) -> Result<String, String> {
        Err("unexpected".into())
    }
    fn onepassword_item(&self, vault: &str, item: &str) -> Result<Value, String> {
        if item != "synthetic-item" {
            return Err("unexpected".into());
        }
        match vault {
            "Private" => Ok(
                json!({"fields":[{"id":"password","value":"default-value"},{"id":"username","value":"named-value"}]}),
            ),
            "other" => Ok(json!({"fields":[{"id":"password","value":"other-vault-value"}]})),
            _ => Err("unexpected".into()),
        }
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        Err("unexpected".into())
    }
}
#[test]
fn item_lookups_default_vault_and_field_without_losing_explicit_values() -> Result<()> {
    let resolver = Lookups {
        resolver: Resolver::new(Arc::new(Fake)),
        values: Mutex::new(Vec::new()),
    };
    for (node, expected) in [
        (
            json!({"kind":"lookup", "provider":"op", "item":"synthetic-item"}),
            "default-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "reference":null}),
            "default-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "field":"username"}),
            "named-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "vault":"other"}),
            "other-vault-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "reference":"op://Private/synthetic-item/username"}),
            "named-value",
        ),
    ] {
        let expression: Expr = serde_json::from_value(node)?;
        assert_eq!(
            evaluate(&expression, &Scope::default(), &resolver)?,
            json!(expected)
        );
    }
    let empty_reference: Expr = serde_json::from_value(
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "reference":""}),
    )?;
    assert!(evaluate(&empty_reference, &Scope::default(), &resolver).is_err());
    Ok(())
}

#[test]
fn overlapping_secrets_are_redacted_independent_of_order() -> Result<()> {
    for secrets in [
        vec!["abc", "abcdef", "cdefg", "aaa"],
        vec!["aaa", "cdefg", "abcdef", "abc"],
    ] {
        let resolver = Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(secrets.into_iter().map(str::to_owned).collect()),
        };
        let input =
            json!({"error":"public: abcdefg / aaaa; suffix stays", "abcdefg":"aaa", "number":42});
        let output = resolver.redact(&input)?;
        assert_eq!(
            output["error"],
            "public: [REDACTED] / [REDACTED]; suffix stays"
        );
        assert_eq!(output["[REDACTED]"], "[REDACTED]");
        assert_eq!(output["number"], 42);
    }
    Ok(())
}

#[test]
fn redaction_handles_unicode_overlaps_and_does_not_rescan_the_marker() -> Result<()> {
    let resolver = Lookups {
        resolver: Resolver::synthetic(),
        values: Mutex::new(vec![
            "é🙂".into(),
            "🙂z".into(),
            "REDACTED".into(),
            String::new(),
        ]),
    };
    assert_eq!(
        resolver.redact(&json!("start é🙂z end"))?,
        json!("start [REDACTED] end")
    );
    Ok(())
}

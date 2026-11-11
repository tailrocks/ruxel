use super::*;
use crate::{
    expression::{Scope, evaluate},
    secrets::Backend,
};
use serde_json::json;
use std::sync::Arc;

mod generated;
struct Fake;
impl Backend for Fake {
    fn onepassword_reference(&self, _: &str) -> Result<String, String> {
        Err("unexpected".into())
    }
    fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
        Err("unexpected".into())
    }
    fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("unexpected".into())
    }
    fn onepassword_fields(
        &self,
        vault: &str,
        item: &str,
        _section: Option<&str>,
        field: &str,
    ) -> Result<Value, String> {
        if item != "synthetic-item" {
            return Err("unexpected".into());
        }
        let value = match (vault, field) {
            ("Private", "password") => "default-value",
            ("Private", "username") => "named-value",
            ("other", "password") => "other-vault-value",
            _ => return Err("unexpected".into()),
        };
        Ok(json!([{ "id": field, "value": value }]))
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        Err("unexpected".into())
    }
}
#[test]
fn item_lookups_require_vault_item_and_field_or_a_reference() -> Result<()> {
    let resolver = Lookups::new(Resolver::new(Arc::new(Fake)).expect("resolver"));
    for (node, expected) in [
        (
            json!({"kind":"lookup", "provider":"op", "vault":"Private", "item":"synthetic-item", "field":"password"}),
            "default-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "vault":"Private", "item":"synthetic-item", "field":"username"}),
            "named-value",
        ),
        (
            json!({"kind":"lookup", "provider":"op", "vault":"other", "item":"synthetic-item", "field":"password"}),
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
    // No default vault or field: a selector that omits either fails rather
    // than reading a value the workflow never named.
    for node in [
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "field":"password"}),
        json!({"kind":"lookup", "provider":"op", "vault":"Private", "item":"synthetic-item"}),
    ] {
        let expression: Expr = serde_json::from_value(node.clone())?;
        assert!(
            evaluate(&expression, &Scope::default(), &resolver).is_err(),
            "{node}"
        );
    }
    let empty_reference: Expr = serde_json::from_value(
        json!({"kind":"lookup", "provider":"op", "item":"synthetic-item", "reference":""}),
    )?;
    assert!(evaluate(&empty_reference, &Scope::default(), &resolver).is_err());
    Ok(())
}

#[test]
fn overlapping_secrets_are_redacted_independent_of_order() {
    for secrets in [
        vec!["abc", "abcdef", "cdefg", "aaa"],
        vec!["aaa", "cdefg", "abcdef", "abc"],
    ] {
        let resolver = Lookups::with_values(
            Resolver::synthetic().expect("synthetic resolver"),
            secrets.into_iter().map(str::to_owned).collect(),
        );
        let input =
            json!({"error":"public: abcdefg / aaaa; suffix stays", "abcdefg":"aaa", "number":42});
        let output = resolver.redact(&input);
        assert_eq!(
            output["error"],
            "public: [REDACTED] / [REDACTED]; suffix stays"
        );
        assert_eq!(output["[REDACTED]"], "[REDACTED]");
        assert_eq!(output["number"], 42);
    }
}

#[test]
fn redaction_handles_unicode_overlaps_and_does_not_rescan_the_marker() {
    let resolver = Lookups::with_values(
        Resolver::synthetic().expect("synthetic resolver"),
        vec!["é🙂".into(), "🙂z".into(), "REDACTED".into(), String::new()],
    );
    assert_eq!(
        resolver.redact(&json!("start é🙂z end")),
        json!("start [REDACTED] end")
    );
}

/// A multi-line value registers each of its lines of 6 bytes or more, with
/// the same derived forms: one line of a fetched document, printed on its
/// own, is censored like the whole value; a line shorter than 6 bytes is
/// not registered on its own and remains covered as part of the whole
/// value.
#[test]
fn a_multi_line_value_registers_each_of_its_lines() {
    let resolver = Lookups::with_values(
        Resolver::synthetic().expect("synthetic resolver"),
        vec!["alpha-secret-line\nbeta-secret-line\n".into()],
    );
    let line = "beta-secret-line";
    let line_wire = "YmV0YS1zZWNyZXQtbGluZQ==";
    let output = resolver.redact(&json!({
        "msg": format!("grep gives {line} and {line_wire}"),
    }));
    assert_eq!(output["msg"], "grep gives [REDACTED] and [REDACTED]");
}

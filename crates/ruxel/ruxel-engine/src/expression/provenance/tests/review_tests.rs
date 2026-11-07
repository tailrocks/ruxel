use super::*;

#[test]
fn missing_secret_reference_taints_default_and_short_array_tree_fails_closed() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"vars":{"secret":{}}, "item":["synthetic-secret"]}),
        provenance: Provenance::Object(BTreeMap::from([
            (
                "vars".into(),
                Provenance::Object(BTreeMap::from([("secret".into(), Provenance::Secret)])),
            ),
            ("item".into(), Provenance::Array(vec![])),
        ])),
    };
    let missing = json!({"kind":"ref", "path":["vars", "secret", "probe"]});
    let fallback = json!({"kind":"literal", "value":"absent"});
    assert!(eval(call("default", &[missing, fallback]), &scope)?.is_secret());
    assert!(eval(json!({"kind":"ref", "path":["item", "0"]}), &scope)?.is_secret());
    Ok(())
}

#[test]
fn wrong_provenance_shape_fails_closed_for_references_and_get() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"item":["synthetic-secret"]}),
        provenance: Provenance::Object(BTreeMap::from([(
            "item".into(),
            Provenance::Object(BTreeMap::new()),
        )])),
    };
    assert!(scope.is_secret());
    assert!(
        scope
            .get("item")
            .and_then(|v| v.get("0"))
            .context("item")?
            .is_secret()
    );
    assert!(eval(json!({"kind":"ref", "path":["item", "0"]}), &scope)?.is_secret());
    Ok(())
}

#[test]
fn provenance_for_absent_object_field_does_not_declassify_default() -> Result<()> {
    let scope = TrackedValue {
        value: json!({"vars":{}}),
        provenance: Provenance::Object(BTreeMap::from([(
            "vars".into(),
            Provenance::Object(BTreeMap::from([("secret".into(), Provenance::Secret)])),
        )])),
    };
    let missing = json!({"kind":"ref", "path":["vars", "secret"]});
    let fallback = json!({"kind":"literal", "value":"absent"});
    assert!(eval(call("default", &[missing, fallback]), &scope)?.is_secret());
    Ok(())
}

#[test]
fn malformed_tree_redacts_output_and_debug() {
    let value = TrackedValue {
        value: json!({"payload":"synthetic-marker"}),
        provenance: Provenance::Object(BTreeMap::from([("absent".into(), Provenance::Secret)])),
    };
    assert_eq!(value.redacted(), json!("[REDACTED]"));
    assert!(!format!("{value:?}").contains("synthetic-marker"));
}

#[test]
fn canonical_vars_root_resolves_without_the_variables_alias() -> Result<()> {
    let plain = crate::expression::Scope {
        variables: json!({"private":"fixture"}),
        ..Default::default()
    };
    let canonical: Expr = serde_json::from_value(json!({"kind":"ref","path":["vars","private"]}))?;
    let alias: Expr = serde_json::from_value(json!({"kind":"ref","path":["variables","private"]}))?;
    assert_eq!(
        crate::expression::evaluate(&canonical, &plain, &Synthetic)?,
        json!("fixture")
    );
    assert!(crate::expression::evaluate(&alias, &plain, &Synthetic).is_err());
    let tracked = TrackedValue::object(BTreeMap::from([(
        "vars".into(),
        TrackedValue::secret(json!({"private":"fixture"})),
    )]));
    assert!(evaluate_tracked(&canonical, &tracked, &Synthetic)?.is_secret());
    assert!(evaluate_tracked(&alias, &tracked, &Synthetic).is_err());
    Ok(())
}

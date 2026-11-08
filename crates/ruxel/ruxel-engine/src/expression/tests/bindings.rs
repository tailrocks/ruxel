use super::*;

#[test]
fn resolves_custom_loop_binding() -> Result<()> {
    let mut scope = Scope::default();
    scope.extra.insert("disk".into(), json!({"name":"data"}));
    assert_eq!(
        run(json!({"kind":"ref", "path":["disk","name"]}), &scope)?,
        json!("data")
    );
    Ok(())
}

#[test]
fn flat_map_rejects_controller_scope_roots() {
    for binding in [
        "item",
        "vars",
        "facts",
        "results",
        "inventoryHost",
        "setFacts",
    ] {
        let expression = call(
            "flatMap",
            vec![lit(json!([1])), lit(json!(binding)), lit(json!([]))],
        );
        assert!(run(expression, &Scope::default()).is_err(), "{binding}");
    }
}

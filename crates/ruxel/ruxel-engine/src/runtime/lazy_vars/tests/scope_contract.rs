use super::*;

#[test]
fn execution_backend_uses_lazy_layered_resolver() -> Result<()> {
    let mut execution = execution()?;
    let parent = json!({"unused":{"kind":"ref","path":["results","future"]},"value":{"kind":"literal","value":"parent"}});
    let child = json!({"value":{"kind":"ref","path":["vars","value"]}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{},"results":{}}));
    let (value, _) = execution.evaluate_layered_expression(
        &json!({"kind":"ref","path":["vars","value"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(value.value, "parent");
    Ok(())
}

#[test]
fn cross_name_override_cycle_matches_frozen_scope() -> Result<()> {
    let execution = execution()?;
    let parent =
        json!({"a":{"kind":"literal","value":"outer"},"b":{"kind":"ref","path":["vars","a"]}});
    let child = json!({"a":{"kind":"call","function":"concat","args":[{"kind":"ref","path":["vars","b"]},{"kind":"literal","value":"-inner"}]}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let expression = json!({"kind":"list","items":[{"kind":"ref","path":["vars","a"]},{"kind":"ref","path":["vars","b"]}]});
    let error = evaluate_layers(&execution, &expression, &scope, &layers).expect_err("cycle");
    assert!(error.to_string().contains("cyclic task variable reference"));
    Ok(())
}

#[test]
fn inherited_alias_observes_and_exports_child_binding() -> Result<()> {
    let execution = execution()?;
    let parent =
        json!({"a":{"kind":"literal","value":"outer"},"b":{"kind":"ref","path":["vars","a"]}});
    let child = json!({"a":{"kind":"literal","value":"inner"}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let (value, bindings) = evaluate_layers(
        &execution,
        &json!({"kind":"ref","path":["vars","b"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(value.value, "inner");
    assert_eq!(bindings["a"].value, "inner");
    assert_eq!(bindings["b"].value, "inner");
    Ok(())
}

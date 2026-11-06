use super::*;

#[tokio::test]
async fn excluded_tasks_do_not_evaluate_unused_vars() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"tagged out","tags":["other"],"vars":{"unused":{"kind":"ref","path":["results","missing"]}},"operation":{"kind":"debug"}},
        {"name":"false condition","tags":["chosen"],"vars":{"unused":{"kind":"ref","path":["results","missing"]},"gate":{"kind":"literal","value":false},"alias":{"kind":"ref","path":["vars","gate"]}},"when":{"kind":"ref","path":["vars","alias"]},"operation":{"kind":"debug"}},
        {"name":"false block","tags":["chosen"],"vars":{"unused":{"kind":"ref","path":["results","missing"]}},"when":false,"block":[{"operation":{"kind":"debug"}}]},
        {"name":"empty loop","tags":["chosen"],"vars":{"unused":{"kind":"ref","path":["results","missing"]},"items":{"kind":"literal","value":[]}},"loop":{"kind":"ref","path":["vars","items"]},"operation":{"kind":"debug"}},
        {"name":"runs","tags":["chosen"],"operation":{"kind":"command","cmd":"ok"}}
    ]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options {
            tags: vec!["chosen".into()],
            ..Options::default()
        },
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    assert_eq!(recap["h"].skipped, 3);
    assert_eq!(recap["h"].ok, 1);
    Ok(())
}

#[tokio::test]
async fn task_override_can_reference_inherited_variable() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"name":{"kind":"literal","value":"parent"}},"tasks":[{"vars":{"name":{"kind":"ref","path":["vars","name"]}},"operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","name"]}}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "parent");
    Ok(())
}

#[tokio::test]
async fn set_fact_takes_precedence_over_task_vars_in_condition() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"operation":{"kind":"set_fact","values":{"gate":{"kind":"literal","value":false}}}},{"vars":{"gate":{"kind":"literal","value":true}},"when":{"kind":"ref","path":["vars","gate"]},"operation":{"kind":"command","cmd":"must not run"}}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert!(backend.calls.is_empty());
    assert_eq!(recap["h"].skipped, 1);
    Ok(())
}

#[test]
fn custom_backend_fallback_resolves_referenced_task_variable() -> Result<()> {
    let mut backend = Fake::default();
    let vars = json!({"value":{"kind":"literal","value":"ready"},"unused":{"kind":"ref","path":["results","missing"]}});
    let expr = json!({"kind":"ref","path":["vars","value"]});
    let scope = TrackedValue::public(json!({"vars":{},"results":{}}));
    let (value, resolved) = super::super::fallback::evaluate(
        &mut backend,
        &expr,
        &scope,
        vars.as_object().context("vars")?,
    )?;
    assert_eq!(value.value, "ready");
    assert_eq!(resolved.len(), 1);
    Ok(())
}

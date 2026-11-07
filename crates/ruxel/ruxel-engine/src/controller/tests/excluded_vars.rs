use super::*;

#[derive(Default)]
struct Opaque(Fake);
impl Backend for Opaque {
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
        self.0.evaluate(value, scope)
    }
    async fn execute(&mut self, request: Request) -> Result<Value> {
        self.0.execute(request).await
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.0.event(event)
    }
}

#[test]
fn opaque_backend_resolves_multilayer_aliases_lazily() -> Result<()> {
    let mut backend = Opaque::default();
    let layers = [
        json!({"x":"parent","alias":{"kind":"ref","path":["vars","x"]},"bad":{"kind":"lookup","provider":"synthetic","key":"unused"}}).as_object().context("parent")?.clone(),
        json!({"x":"child"}).as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let (value, bindings) = backend.evaluate_layered_expression(
        &json!({"kind":"ref","path":["vars","alias"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(value.value, "child");
    assert_eq!(bindings["x"].value, "child");
    assert!(!bindings.contains_key("bad"));
    for definitions in [layers.to_vec(), vec![layers[0].clone()]] {
        let bad = json!({"kind":"ref","path":["vars","bad"]});
        for expression in [
            json!({"kind":"call","function":"and","args":[{"kind":"literal","value":false},bad]}),
            json!({"kind":"call","function":"or","args":[{"kind":"literal","value":true},bad]}),
            json!({"kind":"call","function":"if","args":[{"kind":"literal","value":true},{"kind":"literal","value":true},bad]}),
        ] {
            let (_, resolved) =
                backend.evaluate_layered_expression(&expression, &scope, &definitions)?;
            assert!(resolved.is_empty());
        }
    }
    Ok(())
}

#[test]
fn layered_default_preserves_single_layer_backend_override() -> Result<()> {
    #[derive(Default)]
    struct Override {
        calls: usize,
    }
    impl Backend for Override {
        fn event(&mut self, _: &Value) -> Result<()> {
            Ok(())
        }
        fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
            anyhow::bail!("unexpected evaluation")
        }
        fn evaluate_task_expression(
            &mut self,
            _: &Value,
            _: &TrackedValue,
            _: &Map<String, Value>,
        ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
            self.calls += 1;
            Ok((TrackedValue::secret(json!("override")), BTreeMap::new()))
        }
        fn execute(&mut self, _: Request) -> impl Future<Output = Result<Value>> {
            std::future::ready(Err(anyhow::anyhow!("unexpected execution")))
        }
    }
    let mut backend = Override::default();
    let scope = TrackedValue::public(json!({"vars":{}}));
    let (value, _) = backend.evaluate_layered_expression(&json!(true), &scope, &[Map::new()])?;
    assert_eq!(value.value, "override");
    assert!(value.is_secret());
    assert_eq!(backend.calls, 1);
    Ok(())
}

#[test]
fn layered_standard_backend_keeps_inherited_self_override() -> Result<()> {
    let mut backend = Fake::default();
    let parent = json!({"value":{"kind":"literal","value":"parent"}});
    let child = json!({"value":{"kind":"ref","path":["vars","value"]}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let (value, _) = backend.evaluate_layered_expression(
        &json!({"kind":"ref","path":["vars","value"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(value.value, "parent");
    Ok(())
}

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
    assert_eq!(recap["h"].skipped, 4);
    assert_eq!(backend.events[0]["task"], "tagged out");
    assert_eq!(backend.events[0]["result"]["skipped"], true);
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

#[test]
fn custom_backend_fallback_resolves_shadowed_dependencies_in_order() -> Result<()> {
    let mut backend = Fake::default();
    let vars = json!({"alias":{"kind":"ref","path":["vars","derived"]},"derived":{"kind":"literal","value":"new"}});
    let expr = json!({"kind":"ref","path":["vars","alias"]});
    let scope = TrackedValue::public(json!({"vars":{"derived":"inherited"}}));
    let (value, _) = super::super::fallback::evaluate(
        &mut backend,
        &expr,
        &scope,
        vars.as_object().context("vars")?,
    )?;
    assert_eq!(value.value, "new");
    Ok(())
}

#[test]
fn opaque_lookup_adapter_memoizes_bindings_and_retains_secret_provenance() -> Result<()> {
    #[derive(Default)]
    struct Counter {
        lookups: usize,
    }
    impl Backend for Counter {
        fn event(&mut self, _: &Value) -> Result<()> {
            Ok(())
        }
        fn evaluate(&mut self, value: &Value, _: &Map<String, Value>) -> Result<Value> {
            assert_eq!(value["kind"], "lookup");
            self.lookups += 1;
            Ok(json!("fixture"))
        }
        fn execute(&mut self, _: Request) -> impl Future<Output = Result<Value>> {
            std::future::ready(Err(anyhow::anyhow!("unexpected execution")))
        }
    }
    let mut backend = Counter::default();
    let layers = [
        json!({"private":{"kind":"lookup","provider":"synthetic","key":"fixture"}})
            .as_object()
            .context("parent")?
            .clone(),
        json!({"alias":{"kind":"ref","path":["vars","private"]}})
            .as_object()
            .context("child")?
            .clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let expression = json!({"kind":"list","items":[{"kind":"ref","path":["vars","private"]},{"kind":"ref","path":["vars","alias"]},{"kind":"ref","path":["vars","private"]}]});
    let (value, bindings) = backend.evaluate_layered_expression(&expression, &scope, &layers)?;
    assert_eq!(
        value.redacted(),
        json!(["[REDACTED]", "[REDACTED]", "[REDACTED]"])
    );
    assert!(bindings["private"].is_secret());
    assert!(bindings["alias"].is_secret());
    assert_eq!(backend.lookups, 1);
    Ok(())
}

#[tokio::test]
async fn opaque_backend_runs_inherited_alias_and_skips_unused_bad_binding() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{"source":"parent","alias":{"kind":"ref","path":["vars","source"]},"bad":{"kind":"lookup","provider":"synthetic","key":"unused"}},
        "tasks":[
            {"name":"skipped","vars":{"source":"child"},"when":{"kind":"call","function":"and","args":[{"kind":"literal","value":false},{"kind":"ref","path":["vars","bad"]}]},"operation":{"kind":"command","cmd":"unused"}},
            {"name":"runs","vars":{"source":"child"},"operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","alias"]}}}
        ]
    }]});
    let mut backend = Opaque::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].skipped, 1);
    assert_eq!(recap["h"].ok, 1);
    assert_eq!(backend.0.calls.len(), 1);
    let params: Value = serde_json::from_slice(&backend.0.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "child");
    Ok(())
}

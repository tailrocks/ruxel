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
fn opaque_backend_keeps_custom_evaluation_and_provenance_hooks() -> Result<()> {
    #[derive(Default)]
    struct Hooked(Fake);
    impl Backend for Hooked {
        fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
            self.0.evaluate(value, scope)
        }
        fn evaluate_tracked(
            &mut self,
            value: &Value,
            scope: &TrackedValue,
        ) -> Result<TrackedValue> {
            if let Some(value) = value.get("customValue") {
                return Ok(TrackedValue::secret(value.clone()));
            }
            self.0.evaluate_tracked(value, scope)
        }
        async fn execute(&mut self, request: Request) -> Result<Value> {
            self.0.execute(request).await
        }
        fn event(&mut self, value: &Value) -> Result<()> {
            self.0.event(value)
        }
    }
    let mut backend = Hooked::default();
    let scope = TrackedValue::public(json!({"vars":{}}));
    let parent = json!({"message":{"customValue":"hooked"}})
        .as_object()
        .context("parent")?
        .clone();
    let child = json!({"suffix":"child"})
        .as_object()
        .context("child")?
        .clone();
    for layers in [vec![parent.clone()], vec![parent.clone(), child.clone()]] {
        let (value, _) = backend.evaluate_layered_expression(
            &json!({"kind":"ref","path":["vars","message"]}),
            &scope,
            &layers,
        )?;
        assert_eq!(value.value, "hooked");
        assert!(value.is_secret());
    }
    Ok(())
}

#[test]
fn opaque_backend_keeps_lookup_scope_inside_custom_map() -> Result<()> {
    #[derive(Default)]
    struct Mapped {
        inner: Fake,
        items: Vec<Value>,
    }
    impl Backend for Mapped {
        fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
            if value.get("kind").and_then(Value::as_str) == Some("lookup") {
                let item = scope.get("item").context("item")?.clone();
                self.items.push(item.clone());
                return Ok(item);
            }
            if value.get("function").and_then(Value::as_str) == Some("map") {
                let source = self.evaluate(&value["args"][0], scope)?;
                let mut values = Vec::new();
                for item in source.as_array().context("source")? {
                    let mut child = scope.clone();
                    child.insert("item".into(), item.clone());
                    values.push(self.evaluate(&value["args"][1], &child)?);
                }
                return Ok(Value::Array(values));
            }
            self.inner.evaluate(value, scope)
        }
        async fn execute(&mut self, request: Request) -> Result<Value> {
            self.inner.execute(request).await
        }
        fn event(&mut self, value: &Value) -> Result<()> {
            self.inner.event(value)
        }
    }
    let mut backend = Mapped::default();
    let scope = TrackedValue::public(json!({"vars":{},"item":"outer"}));
    let layers = [
        json!({"parent":"parent"})
            .as_object()
            .context("parent")?
            .clone(),
        json!({"child":"child"})
            .as_object()
            .context("child")?
            .clone(),
    ];
    let expression = json!({"kind":"call","function":"map","args":[{"kind":"literal","value":["first","second"]},{"kind":"lookup","provider":"custom","key":"item"}]});
    let (value, _) = backend.evaluate_layered_expression(&expression, &scope, &layers)?;
    assert_eq!(value.value, json!(["first", "second"]));
    assert_eq!(backend.items, vec![json!("first"), json!("second")]);
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
fn opaque_parent_layers_retain_eager_compatibility_behavior() -> Result<()> {
    let mut backend = Opaque::default();
    let parent = json!({"unused":{"kind":"ref","path":["results","missing"]}})
        .as_object()
        .context("parent")?
        .clone();
    let child = json!({"child":true}).as_object().context("child")?.clone();
    let scope = TrackedValue::public(json!({"vars":{},"results":{}}));
    assert!(
        backend
            .evaluate_layered_expression(&json!(false), &scope, &[parent, child])
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn opaque_variables_materialize_once_at_their_original_scope_boundary() -> Result<()> {
    #[derive(Default)]
    struct Stateful {
        inner: Fake,
        counts: BTreeMap<String, usize>,
    }
    impl Backend for Stateful {
        fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
            self.inner.evaluate(value, scope)
        }
        fn evaluate_tracked(
            &mut self,
            value: &Value,
            scope: &TrackedValue,
        ) -> Result<TrackedValue> {
            if let Some(name) = value.get("customEffect").and_then(Value::as_str) {
                let count = self.counts.entry(name.into()).or_default();
                *count += 1;
                return Ok(TrackedValue::public(json!(format!("{name}-{count}"))));
            }
            self.inner.evaluate_tracked(value, scope)
        }
        async fn execute(&mut self, request: Request) -> Result<Value> {
            self.inner.execute(request).await
        }
        fn event(&mut self, value: &Value) -> Result<()> {
            self.inner.event(value)
        }
    }
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{"playValue":{"customEffect":"play"}},"tasks":[{
            "vars":{"blockValue":{"customEffect":"block"}},"block":[
                {"name":"first","vars":{"local":{"customEffect":"task"}},"when":{"kind":"ref","path":["vars","local"]},"operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","playValue"]}}},
                {"name":"second","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","blockValue"]}}}
            ]
        }]
    }]});
    let mut backend = Stateful::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 2);
    assert_eq!(
        backend.counts,
        BTreeMap::from([("play".into(), 1), ("block".into(), 1), ("task".into(), 1)])
    );
    assert_eq!(backend.inner.events[0]["result"]["msg"], "play-1");
    assert_eq!(backend.inner.events[1]["result"]["msg"], "block-1");
    Ok(())
}

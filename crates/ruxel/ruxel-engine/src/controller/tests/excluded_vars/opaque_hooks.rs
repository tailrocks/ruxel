use super::*;

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

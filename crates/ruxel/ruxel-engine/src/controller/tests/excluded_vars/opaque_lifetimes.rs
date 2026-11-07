use super::*;

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

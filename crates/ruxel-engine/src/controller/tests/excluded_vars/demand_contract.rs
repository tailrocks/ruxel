use super::*;
use crate::expression::{Expr, LookupResolver};
use std::cell::Cell;

#[derive(Default)]
struct DemandBackend {
    inner: Fake,
    lookups: Cell<usize>,
}

impl LookupResolver for DemandBackend {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        self.lookups.set(self.lookups.get() + 1);
        Ok(json!("synthetic-secret"))
    }
}

impl Backend for DemandBackend {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        anyhow::bail!("layered expressions must use the demand-driven contract")
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        evaluate_standard_layered(self, value, scope, layers)
    }
    async fn execute(&mut self, request: Request) -> Result<Value> {
        self.inner.execute(request).await
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.inner.event(event)
    }
}

#[test]
fn demand_driven_backend_ignores_unused_parent_errors_and_effects() -> Result<()> {
    let mut backend = DemandBackend::default();
    let parent = json!({
        "unused":{"kind":"ref","path":["results","missing"]},
        "secret":{"kind":"lookup","provider":"synthetic","key":"unused"}
    });
    let child = json!({"child":{"kind":"literal","value":"ready"}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{},"results":{}}));
    for expression in [json!(false), json!({"kind":"ref","path":["vars","child"]})] {
        let (value, resolved) =
            backend.evaluate_layered_expression(&expression, &scope, &layers)?;
        assert!(!value.is_secret());
        assert!(resolved.keys().all(|name| name == "child"));
    }
    assert_eq!(backend.lookups.get(), 0);
    let (value, _) = backend.evaluate_layered_expression(
        &json!({"kind":"ref","path":["vars","secret"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(value.value, "synthetic-secret");
    assert!(value.is_secret());
    assert_eq!(backend.lookups.get(), 1);
    assert!(
        backend
            .evaluate_layered_expression(
                &json!({"kind":"ref","path":["vars","unused"]}),
                &scope,
                &layers,
            )
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn demand_driven_backend_keeps_unused_play_and_block_bindings_lazy() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{
            "unused":{"kind":"ref","path":["results","missing"]},
            "secret":{"kind":"lookup","provider":"synthetic","key":"unused"}
        },"tasks":[{
            "vars":{"blockUnused":{"kind":"ref","path":["results","missing"]}},
            "block":[
                {"name":"skipped","when":false,"operation":{"kind":"debug"}},
                {"name":"empty","loop":{"kind":"literal","value":[]},"operation":{"kind":"debug"}},
                {"name":"child","vars":{"message":{"kind":"literal","value":"ready"}},
                 "operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","message"]}}}
            ]
        }]
    }]});
    let mut backend = DemandBackend::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recaps["h"].ok, 1);
    assert_eq!(recaps["h"].skipped, 2);
    assert_eq!(backend.lookups.get(), 0);
    assert_eq!(
        backend.inner.events.last().context("event")?["result"]["msg"],
        "ready"
    );
    Ok(())
}

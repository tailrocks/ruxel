use super::*;
use crate::expression::{Expr, LookupResolver};

#[derive(Default)]
struct BackendWithLookup {
    calls: Vec<Request>,
}
impl LookupResolver for BackendWithLookup {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        Ok(json!(true))
    }
}
impl Backend for BackendWithLookup {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        anyhow::bail!("unexpected evaluation")
    }
    fn lazy_variable_layers(&self) -> bool {
        true
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        evaluate_standard_layered(self, value, scope, layers)
    }
    fn execute(&mut self, request: Request) -> impl Future<Output = Result<Value>> {
        self.calls.push(request);
        std::future::ready(Ok(json!({"changed":self.calls.len() > 1, "rc":0})))
    }
    fn event(&mut self, _: &Value) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn current_register_may_change_before_until_and_must_prevent_cache() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"old","register":"run","operation":{"kind":"command","cmd":"one"}},
        {"name":"new","register":"run","retries":0,"until":{"kind":"call","function":"and","args":[
            {"kind":"ref","path":["results","run","changed"]},
            {"kind":"lookup","provider":"synthetic","key":"fixture"}
        ]},"operation":{"kind":"command","cmd":"two"}}
    ]}]});
    let mut backend = BackendWithLookup::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 2);
    assert!(!backend.calls[0].task.no_log);
    assert!(backend.calls[1].task.no_log);
    Ok(())
}

#[tokio::test]
async fn map_rebinds_item_before_until_and_must_prevent_cache() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"loop","loop":{"kind":"list","items":[{"kind":"literal","value":false}]},"retries":0,
         "until":{"kind":"call","function":"contains","args":[
            {"kind":"call","function":"map","args":[{"kind":"list","items":[{"kind":"literal","value":true}]},
                {"kind":"call","function":"and","args":[{"kind":"ref","path":["item"]},{"kind":"lookup","provider":"synthetic","key":"fixture"}]}]},
            {"kind":"literal","value":true}]},"operation":{"kind":"command","cmd":"fixture"}}
    ]}]});
    let mut backend = BackendWithLookup::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    assert!(backend.calls[0].task.no_log);
    Ok(())
}

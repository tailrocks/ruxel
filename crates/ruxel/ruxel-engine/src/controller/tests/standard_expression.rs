use super::*;
use crate::expression::{Expr, LookupResolver};
use std::cell::Cell;

#[derive(Default)]
struct Custom {
    calls: Cell<usize>,
    events: Vec<Value>,
    executions: usize,
}

impl LookupResolver for Custom {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        self.calls.set(self.calls.get() + 1);
        Ok(json!("synthetic"))
    }
}

impl Backend for Custom {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        anyhow::bail!("unexpected opaque evaluation")
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
    fn execute(&mut self, _: Request) -> impl Future<Output = Result<Value>> {
        self.executions += 1;
        std::future::ready(Ok(json!({"changed":false,"rc":0})))
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.events.push(event.clone());
        Ok(())
    }
}

#[tokio::test]
async fn custom_backend_resolves_retry_condition_from_current_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{"status":{"kind":"ref","path":["results","run","rc"]}},
        "tasks":[{"name":"run","register":"run","until":{"kind":"call","function":"eq","args":[{"kind":"ref","path":["vars","status"]},{"kind":"literal","value":0}]},"operation":{"kind":"command","cmd":"fixture"}}]
    }]});
    let mut backend = Custom::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 1);
    assert_eq!(backend.executions, 1);
    assert_eq!(backend.events[0]["result"]["attempts"], 1);
    Ok(())
}

#[tokio::test]
async fn custom_backend_runs_layered_controller_tasks_lazily() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{"name":"parent","alias":{"kind":"ref","path":["vars","name"]},
            "unused":{"kind":"lookup","provider":"synthetic","key":"unused"}},
        "tasks":[{"name":"child","vars":{"name":"child"},"operation":{
            "kind":"debug","msg":{"kind":"ref","path":["vars","alias"]}
        }}]
    }]});
    let mut backend = Custom::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 1);
    assert_eq!(backend.events[0]["result"]["msg"], "child");
    assert_eq!(backend.calls.get(), 0);
    Ok(())
}

#[tokio::test]
async fn custom_backend_resolves_delayed_results_and_updated_facts() -> Result<()> {
    let alias = json!({"kind":"ref","path":["vars","alias"]});
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{
        "targets":["h"],"vars":{"future":{"kind":"ref","path":["results","r","msg"]}},
        "tasks":[
            {"name":"old","operation":{"kind":"set_fact","values":{"source":"old"}}},
            {"name":"block","vars":{"alias":{"kind":"ref","path":["vars","source"]}},
             "when":{"kind":"call","function":"eq","args":[alias,{"kind":"literal","value":"old"}]},"block":[
                {"name":"new","operation":{"kind":"set_fact","values":{"source":"new"}}},
                {"name":"read","register":"r","operation":{"kind":"debug","msg":alias}}
             ]},
            {"name":"future","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","future"]}}}
        ]
    }]});
    let mut backend = Custom::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 4);
    assert_eq!(backend.events[3]["result"]["msg"], "new");
    Ok(())
}

#[test]
fn custom_backend_delegates_lazy_layers_and_lookup_provenance() -> Result<()> {
    let mut backend = Custom::default();
    let parent = json!({"name":"parent","unused":{"kind":"lookup","provider":"synthetic","key":"unused"},"alias":{"kind":"ref","path":["vars","name"]}});
    let child =
        json!({"name":"child","private":{"kind":"lookup","provider":"synthetic","key":"used"}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let expression = json!({"kind":"list","items":[{"kind":"ref","path":["vars","alias"]},{"kind":"ref","path":["vars","private"]},{"kind":"ref","path":["vars","private"]}]});
    let (value, _) = backend.evaluate_layered_expression(&expression, &scope, &layers)?;
    assert_eq!(
        value.redacted(),
        json!(["child", "[REDACTED]", "[REDACTED]"])
    );
    assert_eq!(backend.calls.get(), 1);
    Ok(())
}

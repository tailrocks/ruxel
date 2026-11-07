use super::*;
use crate::expression::{Expr, LookupResolver};
use std::cell::Cell;

#[derive(Default)]
struct Custom {
    calls: Cell<usize>,
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
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        evaluate_standard_layered(self, value, scope, layers)
    }
    fn execute(&mut self, _: Request) -> impl Future<Output = Result<Value>> {
        std::future::ready(Ok(json!({})))
    }
    fn event(&mut self, _: &Value) -> Result<()> {
        Ok(())
    }
}

#[test]
fn custom_backend_delegates_lazy_layers_and_lookup_provenance() -> Result<()> {
    let mut backend = Custom::default();
    let parent = json!({"unused":{"kind":"lookup","provider":"synthetic","key":"unused"},"alias":{"kind":"ref","path":["vars","name"]}});
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

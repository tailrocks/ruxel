use super::*;
#[derive(Default)]
struct FailingPreparation {
    inner: Fake,
    calls: usize,
}
impl Backend for FailingPreparation {
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        self.inner.evaluate_layered_expression(value, scope, layers)
    }
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
        self.inner.evaluate(value, scope)
    }
    fn prepare_operation(
        &mut self,
        _: &str,
        _: &mut Map<String, Value>,
        _: &Map<String, Value>,
    ) -> Result<()> {
        self.calls += 1;
        if self.calls == 2 {
            bail!("synthetic-sensitive-prepare-error");
        }
        Ok(())
    }
    fn execute(&mut self, _: Request) -> impl Future<Output = Result<Value>> {
        std::future::ready(Ok(json!({"changed":false})))
    }
    fn execute_tracked(
        &mut self,
        _: Request,
        _: bool,
    ) -> impl Future<Output = Result<TrackedValue>> {
        std::future::ready(Ok(TrackedValue::secret(json!({"changed":false}))))
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.inner.event(event)
    }
}
#[tokio::test]
async fn sensitive_first_attempt_censors_second_preparation_error() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","retries":1,"delaySeconds":0,"until":{"kind":"literal","value":false},"operation":{"kind":"command","argv":["synthetic"]}}]}]});
    let mut backend = FailingPreparation::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await
    .err()
    .context("expected preparation failure")?;
    assert_eq!(backend.calls, 2);
    assert!(!format!("{error:#}").contains("synthetic-sensitive-prepare-error"));
    assert!(
        !serde_json::to_string(&backend.inner.events)?
            .contains("synthetic-sensitive-prepare-error")
    );
    Ok(())
}

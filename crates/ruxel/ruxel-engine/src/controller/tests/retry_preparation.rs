use super::*;

#[derive(Default)]
struct Changing {
    inner: Fake,
    preparations: usize,
}
impl Backend for Changing {
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
        kind: &str,
        params: &mut Map<String, Value>,
        _: &Map<String, Value>,
    ) -> Result<()> {
        self.preparations += 1;
        let key = if kind == "debug" { "msg" } else { "cmd" };
        params.insert(key.into(), json!(format!("prepared-{}", self.preparations)));
        Ok(())
    }
    fn execute(&mut self, request: Request) -> impl Future<Output = Result<Value>> {
        self.inner.calls.push(request);
        std::future::ready(Ok(json!({"changed":false, "failed":false})))
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.inner.event(event)
    }
}
#[tokio::test]
async fn retries_prepare_remote_and_local_operations_each_time() -> Result<()> {
    for kind in ["command", "debug"] {
        let operation = if kind == "command" {
            json!({"kind":kind, "cmd":"original"})
        } else {
            json!({"kind":kind, "msg":"original"})
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","ignoreErrors":true,"retries":1,"delaySeconds":0,"until":{"kind":"literal","value":false},"operation":operation}]}]});
        let mut backend = Changing::default();
        run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(backend.preparations, 2);
        let result = &backend.inner.events[0]["result"];
        assert_eq!(result["attempts"], 2);
        assert_eq!(result["failed"], true);
        if kind == "command" {
            let values = backend
                .inner
                .calls
                .iter()
                .map(|request| {
                    serde_json::from_slice::<Value>(&request.task.iterations[0].params_json)
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(values[0]["cmd"], "prepared-1");
            assert_eq!(values[1]["cmd"], "prepared-2");
        } else {
            assert_eq!(result["msg"], "prepared-2");
        }
    }
    Ok(())
}
#[tokio::test]
async fn no_until_prepares_only_once() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"once","retries":2,"operation":{"kind":"command","cmd":"original"}}]}]});
    let mut backend = Changing::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.preparations, 1);
    assert_eq!(backend.inner.calls.len(), 1);
    Ok(())
}

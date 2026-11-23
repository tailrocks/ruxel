use super::*;
struct FileBackend {
    inner: Fake,
    source: std::path::PathBuf,
}
impl Backend for FileBackend {
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
        if kind == "template" {
            params.remove("src");
            params.insert(
                "content".into(),
                json!(std::fs::read_to_string(&self.source)?),
            );
        }
        Ok(())
    }
    fn execute(&mut self, request: Request) -> impl Future<Output = Result<Value>> {
        self.inner.calls.push(request);
        std::future::ready((|| {
            if self.inner.calls.len() == 1 {
                std::fs::write(&self.source, "second")?;
            }
            Ok(json!({"ready":self.inner.calls.len() == 2,"changed":false}))
        })())
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.inner.event(event)
    }
}
#[tokio::test]
async fn template_source_is_read_again_after_a_failed_condition() -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("ruxel-retry-source-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let source = directory.join("source");
    std::fs::write(&source, "first")?;
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","retries":1,"delaySeconds":0,"until":{"kind":"ref","path":["result","ready"]},"operation":{"kind":"template","src":"source","dest":"/tmp/synthetic-retry-template"}}]}]});
    let mut backend = FileBackend {
        inner: Fake::default(),
        source,
    };
    let result = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await;
    std::fs::remove_dir_all(&directory)?;
    result?;
    let values = backend
        .inner
        .calls
        .iter()
        .map(|request| serde_json::from_slice::<Value>(&request.task.iterations[0].params_json))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0]["content"], "first");
    assert_eq!(values[1]["content"], "second");
    assert_eq!(backend.inner.events[0]["result"]["attempts"], 2);
    Ok(())
}

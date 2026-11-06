use super::*;

mod cases;
mod excluded_vars;
mod failures;
mod flows;
mod inventory;
mod loop_vars;
mod more;
mod persistence;

#[derive(Default)]
struct Fake {
    calls: Vec<Request>,
    events: Vec<Value>,
    results: std::collections::VecDeque<Value>,
}
impl Backend for Fake {
    fn evaluate_task_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        vars: &Map<String, Value>,
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        let mut scope = scope.clone();
        let mut resolved = BTreeMap::new();
        let names = super::loop_vars::required(vars, value);
        for _ in 0..vars.len() {
            for name in &names {
                if resolved.contains_key(name) {
                    continue;
                }
                if let Some(expr) = vars.get(name)
                    && let Ok(value) = self.evaluate_tracked(expr, &scope)
                {
                    let mut values = scope.get("vars").context("vars")?;
                    values.insert(name.clone(), value.clone())?;
                    scope.insert("vars".into(), values)?;
                    resolved.insert(name.clone(), value);
                }
            }
        }
        Ok((self.evaluate_tracked(value, &scope)?, resolved))
    }
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
        if value.get("kind").and_then(Value::as_str) == Some("ref") {
            let mut current = Value::Object(scope.clone());
            for key in value["path"].as_array().context("path")? {
                current = current
                    .get(key.as_str().context("key")?)
                    .cloned()
                    .context("missing reference")?;
            }
            Ok(current)
        } else if value.get("kind").and_then(Value::as_str) == Some("literal") {
            Ok(value["value"].clone())
        } else if let Some(map) = value.as_object() {
            let mut result = Map::new();
            for (key, item) in map {
                result.insert(key.clone(), self.evaluate(item, scope)?);
            }
            Ok(Value::Object(result))
        } else {
            Ok(value.clone())
        }
    }
    fn execute(&mut self, request: Request) -> impl Future<Output = Result<Value>> {
        self.calls.push(request);
        std::future::ready(Ok(self
            .results
            .pop_front()
            .unwrap_or_else(|| json!({"changed":true}))))
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.events.push(event.clone());
        Ok(())
    }
}

#[tokio::test]
async fn nested_targets_loop_and_registered_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"host":{}},"groups":{"outer":{"children":["inner"]},"inner":{"hosts":["host"]}}},"plays":[{"targets":["outer"],"tasks":[{"name":"loop","loop":["a","b"],"operation":{"kind":"command","cmd":{"kind":"ref","path":["item"]}},"register":"first"},{"name":"next","when":{"kind":"ref","path":["results","first","changed"]},"operation":{"kind":"command","cmd":"next"}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 3);
    assert_eq!(recaps["host"].changed, 2);
    assert_eq!(
        serde_json::from_slice::<Value>(&backend.calls[0].task.iterations[0].params_json)?["cmd"],
        "a"
    );
    Ok(())
}

#[tokio::test]
async fn rescue_always_and_handler_run_in_order() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","block":[{"name":"bad","operation":{"kind":"fail"}}],"rescue":[{"name":"recovered","operation":{"kind":"command","cmd":"rescue"},"notify":["handler"]}],"always":[{"name":"cleanup","operation":{"kind":"command","cmd":"always"}}]}],"handlers":[{"name":"handler","operation":{"kind":"command","cmd":"handler"}}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let names: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.task.name.as_str())
        .collect();
    assert_eq!(names, ["recovered", "cleanup", "handler"]);
    assert_eq!(recap["h"].rescued, 1);
    assert_eq!(recap["h"].failed, 0);
    Ok(())
}

#[tokio::test]
async fn no_log_censors_events_and_diff_checks() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"secret","noLog":true,"operation":{"kind":"command","cmd":"private"}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Diff,
        ..Options::default()
    };
    run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert!(backend.calls[0].task.force_check_mode);
    assert_eq!(backend.events[0]["result"]["censored"], true);
    assert!(!backend.events[0].to_string().contains("private"));
    Ok(())
}

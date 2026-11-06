use super::*;

mod cases;
mod excluded_vars;
mod failures;
mod flows;
mod handler_failure;
mod inventory;
mod loop_vars;
mod more;
mod persistence;
mod plan_redaction;
mod smoke;
mod until_sensitivity;

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
                    && super::loop_vars::required(vars, expr)
                        .iter()
                        .any(|dependency| {
                            dependency != name
                                && vars.contains_key(dependency)
                                && !resolved.contains_key(dependency)
                        })
                {
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

use super::*;

mod always_errors;
mod assertion;
mod block_check_mode;
mod block_controls;
mod block_delegation;
mod block_skip;
mod block_tags;
mod cases;
mod excluded_vars;
mod fact_control;
mod failures;
mod flows;
mod handler_control;
mod handler_failure;
mod handler_validation;
mod inventory;
mod layer_vars;
mod lazy_layers;
mod local_retry;
mod local_status;
mod loop_labels;
mod loop_result;
mod loop_vars;
mod more;
mod persistence;
mod plan_redaction;
mod rescue_recap;
mod result_conditions;
mod retry_branch_privacy;
mod retry_count;
mod retry_preparation;
mod retry_sensitive;
mod retry_source;
mod skipped_result;
mod smoke;
mod standard_expression;
mod until_sensitivity;

#[derive(Default)]
struct Fake {
    conservative_sensitive: bool,
    calls: Vec<Request>,
    events: Vec<Value>,
    results: std::collections::VecDeque<Value>,
}
impl crate::expression::LookupResolver for Fake {
    fn resolve(&self, _: &crate::expression::Expr) -> Result<Value> {
        anyhow::bail!("unexpected lookup")
    }
}
impl Backend for Fake {
    fn conservative_sensitive(&self) -> bool {
        self.conservative_sensitive
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        evaluate_standard_layered(self, value, scope, layers)
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

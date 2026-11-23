use super::*;
use crate::expression::{Expr, LookupResolver};

/// A backend that resolves a lookup to a placeholder, the way the real
/// resolver does in synthetic mode: after the evaluation the selectors
/// are gone, and only the placeholders they resolved to are left.
#[derive(Default)]
struct BackendWithPlaceholders {
    calls: Vec<Request>,
}
impl LookupResolver for BackendWithPlaceholders {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let provider = match expr {
            Expr::Lookup { provider, .. } => provider.clone(),
            _ => "unknown".to_owned(),
        };
        Ok(json!(format!("placeholder for {provider}")))
    }
}
impl Backend for BackendWithPlaceholders {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        anyhow::bail!("unexpected evaluation")
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
        std::future::ready(Ok(json!({"changed": true})))
    }
    fn event(&mut self, _: &Value) -> Result<()> {
        Ok(())
    }
    /// The fingerprint of a secret-bearing content is built from the
    /// selectors alone: the controller asks for it before the content is
    /// evaluated, so the selectors are still in the parameters.
    fn content_fingerprint(&self, params: &Value) -> Option<String> {
        let mut selectors = Vec::new();
        collect_selectors(params.get("content")?, &mut selectors);
        (!selectors.is_empty()).then(|| selectors.join(","))
    }
}

/// Every lookup selector of a value, in field order.
fn collect_selectors(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            if fields.get("kind").and_then(Value::as_str) == Some("lookup")
                && let Some(key) = fields.get("key").and_then(Value::as_str)
            {
                out.push(key.to_owned());
                return;
            }
            for child in fields.values() {
                collect_selectors(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_selectors(child, out);
            }
        }
        _ => {}
    }
}

/// The fingerprint of a secret-bearing file is built from the selectors
/// alone, so the controller reads it from the operation's parameters
/// before they are evaluated: the evaluated parameters carry the
/// placeholders the selectors resolve to, and the selectors are gone.
#[tokio::test]
async fn a_copy_task_carries_the_fingerprint_of_its_selectors() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"Write the env file","operation":{"kind":"copy","dest":"/etc/app/env","content":{"kind":"call","function":"concat","args":[
            {"kind":"literal","value":"PASSWORD="},
            {"kind":"lookup","provider":"fnox","key":"APP_PASSWORD","argv":[]},
            {"kind":"literal","value":"\n"},
            {"kind":"lookup","provider":"fnox","key":"APP_TOKEN","argv":[]}
        ]}}}
    ]}]});
    let mut backend = BackendWithPlaceholders::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    // The evaluated parameters carry the placeholders, not the selectors.
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    let content = params["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no content"))?;
    assert!(
        content.contains("placeholder"),
        "the content is evaluated: {content}"
    );
    assert!(!content.contains("APP_PASSWORD"), "the selector is gone");
    // The fingerprint was built from the selectors, before the evaluation.
    assert_eq!(
        backend.calls[0].content_fingerprint.as_deref(),
        Some("APP_PASSWORD,APP_TOKEN")
    );
    Ok(())
}

use super::*;
use crate::controller::{Backend, Options, Request, evaluate_standard_layered, run};
use crate::expression::{Expr, LookupResolver, provenance::TrackedValue};

#[derive(Default)]
struct OfflineBackend {
    executions: usize,
}

impl LookupResolver for OfflineBackend {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        anyhow::bail!("unexpected lookup")
    }
}

impl Backend for OfflineBackend {
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
    fn execute(&mut self, _: Request) -> impl std::future::Future<Output = Result<Value>> {
        self.executions += 1;
        std::future::ready(Ok(json!({"changed":false,"failed":false})))
    }
    fn event(&mut self, _: &Value) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn dynamic_recurse_is_validated_before_backend_execution() -> Result<()> {
    for state in [
        Value::Null,
        json!("file"),
        json!("absent"),
        json!("link"),
        json!("directory"),
    ] {
        for recursive in [false, true] {
            let mut value = config(
                json!({"kind":"file","path":"/tmp/fixture","src":"/tmp/source",
                "state":{"kind":"ref","path":["vars","state"]},
                "recurse":{"kind":"ref","path":["vars","recursive"]}}),
            );
            value["plays"][0]["vars"] = json!({
                "state":{"kind":"literal","value":state},
                "recursive":{"kind":"literal","value":recursive}
            });
            let validated = validate_config(&value)?;
            let mut backend = OfflineBackend::default();
            let outcome = run(
                &validated,
                &BTreeMap::new(),
                &Options::default(),
                &mut backend,
                &mut BTreeMap::new(),
            )
            .await;
            if recursive && state != "directory" && state != "absent" {
                let error = outcome.expect_err("invalid resolved recurse state");
                assert!(
                    error
                        .to_string()
                        .contains("recurse requires state=directory or state=absent"),
                    "{error:#}"
                );
                assert_eq!(backend.executions, 0);
            } else {
                assert_eq!(outcome?["synthetic"].failed, 0);
                assert_eq!(backend.executions, 1);
            }
        }
    }
    Ok(())
}

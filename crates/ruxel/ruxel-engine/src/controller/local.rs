use super::{
    Backend, Context, ContextLayer, Mode, Result, Runner, TrackedValue, Value, bail, json, truth,
};
use serde_json::Map;

impl<B: Backend> Runner<'_, B> {
    #[expect(
        clippy::too_many_lines,
        reason = "local operations share controller dispatch"
    )]
    pub(super) async fn local_once(
        &mut self,
        task: &Value,
        kind: &str,
        layer: &ContextLayer,
        params: &Map<String, Value>,
        tracked_params: &TrackedValue,
        sensitive: bool,
    ) -> Result<TrackedValue> {
        let result_value = |value| {
            if sensitive {
                TrackedValue::secret(value)
            } else {
                TrackedValue::public(value)
            }
        };
        match kind {
            "pause" => {
                if (matches!(self.options.mode, Mode::Check | Mode::Diff)
                    || task.get("checkMode").and_then(Value::as_bool) == Some(true))
                    && task.get("checkMode").and_then(Value::as_bool) != Some(false)
                {
                    return Ok(result_value(
                        json!({"changed":false,"failed":false,"skipped":true}),
                    ));
                }
                let seconds = params.get("seconds").and_then(Value::as_u64).unwrap_or(0);
                let minutes = params.get("minutes").and_then(Value::as_u64).unwrap_or(0);
                let duration = seconds
                    .checked_add(minutes.checked_mul(60).context("pause duration overflow")?)
                    .context("pause duration overflow")?;
                if duration > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(duration)).await;
                } else {
                    use std::io::IsTerminal;
                    let prompt = params
                        .get("prompt")
                        .and_then(Value::as_str)
                        .unwrap_or("Press Enter to continue");
                    self.backend
                        .event(&json!({"host":self.host,"pause":if layer.no_log || sensitive { "[censored]" } else { prompt }}))?;
                    if std::io::stdin().is_terminal() {
                        let _ = tokio::task::spawn_blocking(|| {
                            let mut line = String::new();
                            std::io::stdin().read_line(&mut line)
                        })
                        .await??;
                    }
                }
                Ok(result_value(
                    json!({"changed":false,"failed":false,"user_input":""}),
                ))
            }
            "debug" => Ok(result_value(
                json!({"changed":false, "failed":false, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Hello world!"))}),
            )),
            "fail" => Ok(result_value(
                json!({"changed":false, "failed":true, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Failed as requested"))}),
            )),
            "assert" => {
                let assertion = params.get("that").context("assert requires that")?;
                let pass = match assertion {
                    Value::Array(values) => values.iter().all(truth),
                    value => truth(value),
                };
                Ok(result_value(
                    json!({"changed":false,"failed":!pass,"msg":if pass { json!("All assertions passed") } else { params.get("fail_msg").cloned().unwrap_or_else(|| json!("Assertion failed")) }}),
                ))
            }
            "set_fact" => {
                let values = params
                    .get("values")
                    .and_then(Value::as_object)
                    .context("set_fact requires values")?;
                let mut tracked_values = tracked_params
                    .get("values")
                    .context("set_fact requires values")?;
                if layer.no_log || layer.control_sensitive {
                    tracked_values.provenance = crate::expression::provenance::Provenance::Secret;
                }
                let mut facts = self
                    .scope
                    .get("setFacts")
                    .unwrap_or_else(|| TrackedValue::public(json!({})));
                for key in values.keys() {
                    facts.insert(
                        key.clone(),
                        tracked_values.get(key).context("fact missing")?,
                    )?;
                }
                self.scope.insert("setFacts".into(), facts)?;
                Ok(result_value(
                    json!({"changed":false,"failed":false,"facts":values}),
                ))
            }
            _ => bail!("unsupported local operation"),
        }
    }
}

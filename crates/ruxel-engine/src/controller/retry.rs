use super::{Context, Result, Value};

pub(super) fn max_attempts(task: &Value) -> Result<u64> {
    let retries = match task.get("retries") {
        None | Some(Value::Null) => 0,
        Some(value) => value.as_u64().context("retries must be nonnegative")?,
    };
    if task.get("until").is_none_or(Value::is_null) {
        return Ok(1);
    }
    retries
        .checked_add(1)
        .context("retries exceed supported range")
}

pub(super) fn delay_seconds(task: &Value) -> u64 {
    task.get("delaySeconds")
        .and_then(Value::as_u64)
        .unwrap_or(5)
}

impl<B: super::Backend> super::Runner<'_, B> {
    pub(super) async fn execute(
        &mut self,
        task: &Value,
        layer: &super::ContextLayer,
        scope: &super::TrackedValue,
        item: &super::TrackedValue,
    ) -> Result<super::TrackedValue> {
        let Some(condition) = task.get("until").filter(|v| !v.is_null()) else {
            return self.execute_once(task, layer, scope, item).await;
        };
        if self.options.mode == super::Mode::Plan {
            return self.execute_once(task, layer, scope, item).await;
        }
        let attempts = max_attempts(task)?;
        let mut layer = layer.clone();
        layer.control_sensitive |= self.sensitive_layered_condition(condition, scope, &layer);
        let mut changed = false;
        for attempt in 1..=attempts {
            let tracked = self.execute_once(task, &layer, scope, item).await?;
            layer.control_sensitive |= tracked.is_secret();
            let mut result = tracked.value;
            if !result.is_object() {
                anyhow::bail!("operation result must be an object");
            }
            changed |= result.get("changed").and_then(Value::as_bool) == Some(true);
            let current = if layer.control_sensitive || layer.no_log {
                super::TrackedValue::secret(result.clone())
            } else {
                super::TrackedValue::public(result.clone())
            };
            let current_scope = super::result_scope(scope, task, &current)?;
            let evaluated = self
                .evaluate_scoped(condition, &current_scope, &layer)
                .map_err(|error| {
                    if layer.control_sensitive || layer.no_log {
                        anyhow::anyhow!("sensitive retry condition failed")
                    } else {
                        error
                    }
                })?;
            layer.control_sensitive |= evaluated.is_secret();
            let done = super::truth(&evaluated.value);
            if done || attempt == attempts {
                if !done {
                    result["failed"] = Value::Bool(true);
                }
                if changed {
                    result["changed"] = Value::Bool(true);
                }
                result["attempts"] = Value::from(attempt);
                return Ok(if layer.control_sensitive || layer.no_log {
                    super::TrackedValue::secret(result)
                } else {
                    super::TrackedValue::public(result)
                });
            }
            tokio::time::sleep(std::time::Duration::from_secs(delay_seconds(task))).await;
        }
        anyhow::bail!("retry loop did not execute")
    }
}

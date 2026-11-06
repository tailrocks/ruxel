use super::{
    Backend, Context, ContextLayer, Map, Mode, Request, Result, Runner, Value, bail, failed, json,
    ledger_key, result_scope, snake, string, truth, v1,
};

impl<B: Backend> Runner<'_, B> {
    #[expect(
        clippy::too_many_lines,
        reason = "operation preparation and retry share scope"
    )]
    pub(super) async fn execute(
        &mut self,
        task: &Value,
        layer: &ContextLayer,
        scope: &Map<String, Value>,
        item: &Value,
    ) -> Result<Value> {
        let operation = task
            .get("operation")
            .filter(|v| !v.is_null())
            .context("task has no operation")?;
        let kind = string(operation, "kind").context("operation has no kind")?;
        let mut params = Map::new();
        for (key, value) in operation
            .as_object()
            .context("operation must be an object")?
        {
            if key != "kind" && !value.is_null() {
                params.insert(snake(key), value.clone());
            }
        }
        self.backend.prepare_operation(kind, &mut params, scope)?;
        let params = self.backend.evaluate(&Value::Object(params), scope)?;
        let params = params
            .as_object()
            .context("operation parameters must evaluate to an object")?
            .clone();
        crate::model::validate_operation(kind, &params)?;
        match kind {
            "pause" => {
                if (matches!(self.options.mode, Mode::Check | Mode::Diff)
                    || task.get("checkMode").and_then(Value::as_bool) == Some(true))
                    && task.get("checkMode").and_then(Value::as_bool) != Some(false)
                {
                    return Ok(json!({"changed":false,"skipped":true}));
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
                        .event(&json!({"host":self.host,"pause":if layer.no_log { "[censored]" } else { prompt }}))?;
                    if std::io::stdin().is_terminal() {
                        let _ = tokio::task::spawn_blocking(|| {
                            let mut line = String::new();
                            std::io::stdin().read_line(&mut line)
                        })
                        .await??;
                    }
                }
                return Ok(json!({"changed":false,"user_input":""}));
            }
            "debug" => {
                return Ok(
                    json!({"changed":false, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Hello world!"))}),
                );
            }
            "fail" => {
                return Ok(
                    json!({"changed":false, "failed":true, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Failed as requested"))}),
                );
            }
            "assert" => {
                let assertion = params.get("that").context("assert requires that")?;
                let pass = match assertion {
                    Value::Array(values) => values.iter().all(truth),
                    value => truth(value),
                };
                return Ok(
                    json!({"changed":false,"failed":!pass,"msg":if pass { json!("All assertions passed") } else { params.get("fail_msg").cloned().unwrap_or_else(|| json!("Assertion failed")) }}),
                );
            }
            "set_fact" => {
                let values = params
                    .get("values")
                    .and_then(Value::as_object)
                    .context("set_fact requires values")?;
                self.scope
                    .entry("setFacts")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .context("results scope is not an object")?
                    .extend(values.clone());
                return Ok(json!({"changed":false,"facts":values}));
            }
            _ => {}
        }
        let mut environment = std::collections::HashMap::new();
        for (key, value) in &layer.environment {
            let value = self.backend.evaluate(value, scope)?;
            environment.insert(
                key.clone(),
                value
                    .as_str()
                    .context("environment value must be a string")?
                    .to_owned(),
            );
        }
        let destination = if let Some(delegate) = task.get("delegateTo").filter(|v| !v.is_null()) {
            self.backend
                .evaluate(delegate, scope)?
                .as_str()
                .context("delegateTo must evaluate to a host name")?
                .to_owned()
        } else {
            self.host.clone()
        };
        let id = self.next_id;
        self.next_id += 1;
        let item_label = if layer.no_log {
            "[censored]".to_owned()
        } else if let Some(label) = task.get("loopLabel").filter(|v| !v.is_null()) {
            let value = self.backend.evaluate(label, scope)?;
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        } else if item.is_null() {
            String::new()
        } else {
            item.to_string()
        };
        let free_form = if kind == "shell" {
            params
                .get("cmd")
                .and_then(Value::as_str)
                .context("shell requires a string cmd")?
                .to_owned()
        } else {
            String::new()
        };
        let mut request = Request {
            host: destination,
            task: v1::RenderedTask {
                task_id: id,
                name: string(task, "name").unwrap_or(kind).to_owned(),
                module: kind.to_owned(),
                rendered: true,
                iterations: vec![v1::Iteration {
                    item_label,
                    params_json: serde_json::to_vec(&params)?,
                    free_form,
                    ledger_key: String::new(),
                    source_content: None,
                }],
                check_mode_override: task.get("checkMode").and_then(Value::as_bool) == Some(false),
                no_log: layer.no_log,
                become_user: layer.become_user.clone().unwrap_or_default(),
                environment,
                halt_on_failure: true,
                force_check_mode: matches!(self.options.mode, Mode::Check | Mode::Diff)
                    || task.get("checkMode").and_then(Value::as_bool) == Some(true),
            },
        };
        let key = ledger_key(&request.task, &request.task.iterations[0])?;
        request.task.iterations[0].ledger_key = key;
        if self.options.mode == Mode::Plan {
            let visible_params = if layer.no_log {
                json!({"censored":true})
            } else {
                Value::Object(params)
            };
            self.backend.event(&json!({"host":request.host,"task":request.task.name,"module":kind,"params":visible_params,"planned":true}))?;
            return Ok(json!({"changed":false,"planned":true}));
        }
        let attempts = task
            .get("retries")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let mut result = Value::Null;
        for attempt in 0..attempts {
            result = self.backend.execute(request.clone()).await?;
            if !result.is_object() {
                bail!("operation result must be an object");
            }
            let result_scope = result_scope(scope, task, &result)?;
            let done = match task.get("until").filter(|v| !v.is_null()) {
                Some(expr) => truth(&self.backend.evaluate(expr, &result_scope)?),
                None => !failed(&result),
            };
            if done {
                break;
            }
            if attempt + 1 == attempts {
                result["failed"] = Value::Bool(true);
            }
            if attempt + 1 < attempts {
                tokio::time::sleep(std::time::Duration::from_secs(
                    task.get("delaySeconds")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                ))
                .await;
            }
        }
        Ok(result)
    }
}

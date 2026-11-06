use super::{
    Backend, Context, ContextLayer, Map, Mode, Request, Result, Runner, TrackedValue, Value, bail,
    failed, json, ledger_key, result_scope, snake, string, truth, v1,
};

pub(super) fn sensitive_condition(value: &Value, scope: &TrackedValue) -> bool {
    match value {
        Value::Object(fields) if fields.get("kind").and_then(Value::as_str) == Some("ref") => {
            let Some(path) = fields.get("path").and_then(Value::as_array) else {
                return false;
            };
            let mut current = scope.clone();
            for (index, part) in path.iter().enumerate() {
                let Some(mut part) = part.as_str() else {
                    return false;
                };
                if index == 0 && part == "variables" {
                    part = "vars";
                }
                let Some(next) = current.get(part) else {
                    return matches!(
                        current.provenance,
                        crate::expression::provenance::Provenance::Secret
                    );
                };
                current = next;
            }
            current.is_secret()
        }
        Value::Object(fields) => {
            fields.get("kind").and_then(Value::as_str) == Some("lookup")
                || fields
                    .values()
                    .any(|child| sensitive_condition(child, scope))
        }
        Value::Array(items) => items.iter().any(|child| sensitive_condition(child, scope)),
        _ => false,
    }
}

impl<B: Backend> Runner<'_, B> {
    #[expect(
        clippy::too_many_lines,
        reason = "operation preparation and retry share scope"
    )]
    pub(super) async fn execute(
        &mut self,
        task: &Value,
        layer: &ContextLayer,
        scope: &TrackedValue,
        item: &TrackedValue,
    ) -> Result<TrackedValue> {
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
        let mut source_sensitive = false;
        if kind == "template"
            && params.get("content").is_none_or(Value::is_null)
            && let Some(source) = params.get("src")
        {
            let source = self.backend.evaluate_tracked(source, scope)?;
            source_sensitive = source.is_secret();
            params.insert("src".into(), source.value);
        }
        source_sensitive = self
            .backend
            .prepare_operation_tracked(
                kind,
                &mut params,
                scope.value.as_object().context("scope must be an object")?,
                source_sensitive || layer.no_log || layer.control_sensitive,
            )
            .await?;
        let tracked_params = self
            .backend
            .evaluate_tracked(&Value::Object(params), scope)
            .map_err(|error| {
                if source_sensitive {
                    anyhow::anyhow!("sensitive operation evaluation failed")
                } else {
                    error
                }
            })?;
        let mut sensitive = layer.no_log
            || layer.control_sensitive
            || source_sensitive
            || tracked_params.is_secret();
        if let Some(condition) = task.get("until").filter(|value| !value.is_null()) {
            sensitive |= sensitive_condition(condition, scope);
        }
        let params = tracked_params
            .value
            .as_object()
            .context("operation parameters must evaluate to an object")?
            .clone();
        crate::model::validate_operation(kind, &params).map_err(|e| {
            if sensitive {
                anyhow::anyhow!("sensitive operation validation failed")
            } else {
                e
            }
        })?;
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
                    return Ok(result_value(json!({"changed":false,"skipped":true})));
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
                return Ok(result_value(json!({"changed":false,"user_input":""})));
            }
            "debug" => {
                return Ok(result_value(
                    json!({"changed":false, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Hello world!"))}),
                ));
            }
            "fail" => {
                return Ok(result_value(
                    json!({"changed":false, "failed":true, "msg":params.get("msg").cloned().unwrap_or_else(|| json!("Failed as requested"))}),
                ));
            }
            "assert" => {
                let assertion = params.get("that").context("assert requires that")?;
                let pass = match assertion {
                    Value::Array(values) => values.iter().all(truth),
                    value => truth(value),
                };
                return Ok(result_value(
                    json!({"changed":false,"failed":!pass,"msg":if pass { json!("All assertions passed") } else { params.get("fail_msg").cloned().unwrap_or_else(|| json!("Assertion failed")) }}),
                ));
            }
            "set_fact" => {
                let values = params
                    .get("values")
                    .and_then(Value::as_object)
                    .context("set_fact requires values")?;
                let mut tracked_values = tracked_params
                    .get("values")
                    .context("set_fact requires values")?;
                if layer.no_log {
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
                return Ok(result_value(json!({"changed":false,"facts":values})));
            }
            _ => {}
        }
        let mut environment = std::collections::HashMap::new();
        for (key, value) in &layer.environment {
            let value = self.backend.evaluate_tracked(value, scope)?;
            sensitive |= value.is_secret();
            environment.insert(
                key.clone(),
                value
                    .value
                    .as_str()
                    .context("environment value must be a string")?
                    .to_owned(),
            );
        }
        let destination = if let Some(delegate) = task.get("delegateTo").filter(|v| !v.is_null()) {
            let value = self.backend.evaluate_tracked(delegate, scope)?;
            sensitive |= value.is_secret();
            value
                .value
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
            let value = self.backend.evaluate_tracked(label, scope)?;
            sensitive |= value.is_secret();
            value
                .value
                .as_str()
                .map_or_else(|| value.value.to_string(), str::to_owned)
        } else if item.value.is_null() {
            String::new()
        } else {
            sensitive |= item.is_secret();
            item.value.to_string()
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
                no_log: sensitive,
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
            let visible_params = if sensitive {
                json!({"censored":true})
            } else {
                tracked_params.redacted()
            };
            let visible_host = if sensitive { &self.host } else { &request.host };
            self.backend.event(&json!({"host":visible_host,"task":request.task.name,"module":kind,"params":visible_params,"planned":true}))?;
            return Ok(if sensitive {
                TrackedValue::secret(json!({"changed":false,"planned":true}))
            } else {
                TrackedValue::public(json!({"changed":false,"planned":true}))
            });
        }
        let attempts = task
            .get("retries")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let mut result = Value::Null;
        let mut changed = false;
        let mut executed = 0;
        for attempt in 0..attempts {
            let tracked = self
                .backend
                .execute_tracked(request.clone(), sensitive)
                .await?;
            sensitive |= tracked.is_secret();
            result = tracked.value;
            executed = attempt + 1;
            if !result.is_object() {
                bail!("operation result must be an object");
            }
            changed |= result.get("changed").and_then(Value::as_bool) == Some(true);
            let result_scope = result_scope(
                scope,
                task,
                &if sensitive {
                    TrackedValue::secret(result.clone())
                } else {
                    TrackedValue::public(result.clone())
                },
            )?;
            let done = match task.get("until").filter(|v| !v.is_null()) {
                Some(expr) => {
                    let condition =
                        self.backend
                            .evaluate_tracked(expr, &result_scope)
                            .map_err(|error| {
                                if sensitive {
                                    anyhow::anyhow!("sensitive retry condition failed")
                                } else {
                                    error
                                }
                            })?;
                    sensitive |= condition.is_secret();
                    request.task.no_log |= sensitive;
                    truth(&condition.value)
                }
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
        if changed {
            result["changed"] = Value::Bool(true);
        }
        if task.get("until").is_some_and(|value| !value.is_null()) {
            result["attempts"] = Value::from(executed);
        }
        Ok(if sensitive {
            TrackedValue::secret(result)
        } else {
            TrackedValue::public(result)
        })
    }
}

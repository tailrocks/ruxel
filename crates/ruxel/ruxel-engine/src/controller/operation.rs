use super::{
    Backend, Context, ContextLayer, Map, Mode, Request, Result, Runner, TrackedValue, Value, bail,
    json, ledger_key, snake, string, v1,
};

pub(super) fn sensitive_condition(value: &Value, scope: &TrackedValue) -> bool {
    match value {
        Value::Object(fields) if fields.get("kind").and_then(Value::as_str) == Some("ref") => {
            let Some(path) = fields.get("path").and_then(Value::as_array) else {
                return false;
            };
            let mut current = scope.clone();
            for part in path {
                let Some(part) = part.as_str() else {
                    return false;
                };
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
    pub(super) async fn execute_once(
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
        // FetchSecret is apply-only: in check and diff it contacts neither
        // fnox nor the host, and reports known-after-apply rather than a
        // fingerprint comparison. The comparison and the store happen only
        // in apply.
        if kind == "fetchSecret" && matches!(self.options.mode, Mode::Check | Mode::Diff) {
            return Ok(TrackedValue::public(json!(
                {"changed": false, "failed": false, "knownAfterApply": true}
            )));
        }
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
            let source = self.evaluate_scoped(source, scope, layer)?;
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
        let tracked_params = if kind == "assert" {
            self.assertion_params(
                &params,
                scope,
                layer,
                source_sensitive || layer.no_log || layer.control_sensitive,
            )
        } else {
            self.evaluate_scoped(&Value::Object(params), scope, layer)
        }
        .map_err(|error| {
            if source_sensitive || layer.no_log || layer.control_sensitive {
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
            sensitive |= self.sensitive_layered_condition(condition, scope, layer);
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
        if matches!(kind, "pause" | "debug" | "fail" | "assert" | "set_fact") {
            return self
                .local_once(kind, layer, &params, &tracked_params, sensitive)
                .await;
        }
        let mut environment = std::collections::HashMap::new();
        for (key, value) in &layer.environment {
            let value = self.evaluate_scoped(value, scope, layer)?;
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
        let destination = if let Some(delegate) = &layer.delegate_to {
            let value = self.evaluate_scoped(delegate, scope, layer)?;
            sensitive |= value.is_secret();
            let destination = value
                .value
                .as_str()
                .context("delegateTo must evaluate to a host name")?
                .to_owned();
            anyhow::ensure!(
                layer
                    .delegate_allow
                    .iter()
                    .any(|allowed| allowed == &destination),
                "delegation target is not explicitly authorized"
            );
            anyhow::ensure!(
                destination != "localhost" || layer.delegate_localhost,
                "localhost delegation requires explicit opt-in"
            );
            destination
        } else {
            self.host.clone()
        };
        let id = self.next_id;
        self.next_id += 1;
        let item_label = if layer.no_log {
            "[censored]".to_owned()
        } else if let Some(label) = task.get("loopLabel").filter(|v| !v.is_null()) {
            let value = self.evaluate_scoped(label, scope, layer)?;
            sensitive |= value.is_secret();
            if value.is_secret() {
                "[censored]".to_owned()
            } else {
                value
                    .value
                    .as_str()
                    .map_or_else(|| value.value.to_string(), str::to_owned)
            }
        } else if item.value.is_null() {
            String::new()
        } else {
            sensitive |= item.is_secret();
            if item.is_secret() {
                "[censored]".to_owned()
            } else {
                item.value.to_string()
            }
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
                check_mode_override: layer.check_mode == Some(false),
                no_log: sensitive,
                become_user: layer.become_user.clone().unwrap_or_default(),
                environment,
                halt_on_failure: true,
                force_check_mode: matches!(self.options.mode, Mode::Check | Mode::Diff)
                    || layer.check_mode == Some(true),
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
        let result = self.backend.execute_tracked(request, sensitive).await?;
        if !result.value.is_object() {
            bail!("operation result must be an object");
        }
        Ok(result)
    }
}

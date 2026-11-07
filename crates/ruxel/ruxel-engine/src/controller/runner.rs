use super::{
    Backend, Context, ContextLayer, Future, Pin, Result, Runner, TrackedValue, Value, array, bail,
    failed, flag, json, object, result_scope, string, truth,
};

impl<B: Backend> Runner<'_, B> {
    fn layer(task: &Value, parent: &ContextLayer) -> ContextLayer {
        let mut layer = parent.clone();
        let definitions = object(task, "vars");
        for name in definitions.keys() {
            layer.resolved.remove(name);
        }
        if !definitions.is_empty() {
            layer.definitions.push(definitions);
        }
        layer.environment.extend(object(task, "environment"));
        layer.no_log |= flag(task, "noLog");
        if let Some(user) = string(task, "becomeUser") {
            layer.become_user = Some(user.to_owned());
        }
        layer.tags.extend(
            array(task, "tags")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
        layer
    }

    fn evaluate_selection(
        &mut self,
        _task: &Value,
        layer: &mut ContextLayer,
        item: Option<(&str, &TrackedValue)>,
        expression: &Value,
        resolved: &mut std::collections::BTreeSet<String>,
    ) -> Result<TrackedValue> {
        let definitions = self.variable_layers(layer);
        let scope = self.scope(layer, item);
        let (value, vars) =
            self.backend
                .evaluate_layered_expression(expression, &scope, &definitions)?;
        for (name, value) in vars {
            resolved.insert(name.clone());
            layer.resolved.insert(name.clone());
            layer.vars.insert(name, value);
        }
        Ok(value)
    }

    pub(super) fn scope(
        &self,
        layer: &ContextLayer,
        item: Option<(&str, &TrackedValue)>,
    ) -> TrackedValue {
        let mut fields = std::collections::BTreeMap::new();
        if let Some(existing) = self.scope.value.as_object() {
            for key in existing.keys() {
                if let Some(value) = self.scope.get(key) {
                    fields.insert(key.clone(), value);
                }
            }
        }
        let mut vars = layer.vars.clone();
        if let Some(facts) = self.scope.get("setFacts")
            && let Some(map) = facts.value.as_object()
        {
            for key in map.keys() {
                if let Some(value) = facts.get(key) {
                    vars.insert(key.clone(), value);
                }
            }
        }
        fields.insert("vars".into(), TrackedValue::object(vars));
        if let Some((name, value)) = item {
            fields.insert(name.to_owned(), value.clone());
            fields.insert("item".into(), value.clone());
        }
        TrackedValue::object(fields)
    }

    pub(super) fn tasks<'a>(
        &'a mut self,
        tasks: &'a [Value],
        layer: &'a ContextLayer,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + 'a>> {
        Box::pin(async move {
            for task in tasks {
                let result = self.task(task, layer).await.map_err(|error| {
                    if layer.no_log || flag(task, "noLog") || layer.control_sensitive {
                        anyhow::anyhow!("sensitive task failed")
                    } else {
                        error
                    }
                })?;
                if !result {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    pub(super) fn task<'a>(
        &'a mut self,
        task: &'a Value,
        parent: &'a ContextLayer,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + 'a>> {
        Box::pin(async move {
            if !self.handling
                && !self.options.tags.is_empty()
                && !super::tag_selection::selected(task, &parent.tags, &self.options.tags)
            {
                if array(task, "block").is_empty() {
                    self.skip_tagged_leaf(task, parent)?;
                }
                return Ok(true);
            }
            let mut layer = Self::layer(task, parent);
            if !array(task, "block").is_empty() {
                let mut resolved = std::collections::BTreeSet::new();
                if let Some(condition) = task.get("when").filter(|v| !v.is_null()) {
                    let condition =
                        self.evaluate_selection(task, &mut layer, None, condition, &mut resolved)?;
                    layer.control_sensitive |= condition.is_secret();
                    if !truth(&condition.value) {
                        for child in array(task, "block") {
                            self.recap.skipped += 1;
                            let result = if layer.no_log
                                || flag(child, "noLog")
                                || layer.control_sensitive
                            {
                                json!({"censored":true,"changed":false,"failed":false})
                            } else {
                                json!({"skipped":true,"changed":false})
                            };
                            self.backend.event(&json!({"host":self.host,"task":string(child,"name").unwrap_or(""),"result":result}))?;
                        }
                        return Ok(true);
                    }
                }
                let mut success = self.tasks(array(task, "block"), &layer).await?;
                if !success && !array(task, "rescue").is_empty() {
                    self.recap.rescued += 1;
                    self.recap.failed = self.recap.failed.saturating_sub(1);
                    success = self.tasks(array(task, "rescue"), &layer).await?;
                }
                let always = self.tasks(array(task, "always"), &layer).await?;
                return Ok(success && always);
            }
            self.operation(task, &layer).await
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "iteration and recap form one task execution"
    )]
    pub(super) async fn operation(&mut self, task: &Value, layer: &ContextLayer) -> Result<bool> {
        let mut loop_layer = layer.clone();
        let mut loop_source_sensitive = false;
        let items = if let Some(expr) = task.get("loop").filter(|v| !v.is_null()) {
            let value = self.evaluate_selection(
                task,
                &mut loop_layer,
                None,
                expr,
                &mut std::collections::BTreeSet::new(),
            )?;
            loop_source_sensitive = layer.no_log || layer.control_sensitive || value.is_secret();
            let items = value
                .value
                .as_array()
                .context("loop must evaluate to a list")?;
            (0..items.len())
                .map(|i| value.get(&i.to_string()).context("loop item missing"))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![TrackedValue::public(Value::Null)]
        };
        let looped = task.get("loop").is_some_and(|v| !v.is_null());
        let mut results = Vec::new();
        for item in &items {
            let binding = looped.then_some((string(task, "loopVar").unwrap_or("item"), item));
            let mut iteration_layer = layer.clone();
            iteration_layer.control_sensitive |= looped && item.is_secret();
            let mut resolved = std::collections::BTreeSet::new();
            if let Some(condition) = task.get("when").filter(|v| !v.is_null()) {
                let condition = self.evaluate_selection(
                    task,
                    &mut iteration_layer,
                    binding,
                    condition,
                    &mut resolved,
                )?;
                iteration_layer.control_sensitive |= condition.is_secret();
                if !truth(&condition.value) {
                    let skipped = json!({"skipped":true,"changed":false,"failed":false,"skip_reason":"Conditional result was False"});
                    let mut skipped = if iteration_layer.control_sensitive || iteration_layer.no_log
                    {
                        TrackedValue::secret(skipped)
                    } else {
                        TrackedValue::public(skipped)
                    };
                    if looped {
                        skipped.insert("item".into(), item.clone())?;
                    }
                    results.push(skipped);
                    continue;
                }
            }
            let scope = self.scope(&iteration_layer, binding);
            let mut result = self
                .execute(task, &iteration_layer, &scope, item)
                .await
                .map_err(|error| {
                    if iteration_layer.control_sensitive {
                        anyhow::anyhow!("sensitive task failed")
                    } else {
                        error
                    }
                })?;
            if !result.value.is_object() {
                bail!("operation result must be an object");
            }
            if looped {
                result.insert("item".into(), item.clone())?;
            }
            let current_scope = result_scope(&scope, task, &result)?;
            if let Some(expr) = task.get("failedWhen").filter(|v| !v.is_null()) {
                let condition = self.evaluate_scoped(expr, &current_scope, &iteration_layer)?;
                let outcome = if condition.is_secret() {
                    TrackedValue::secret(Value::Bool(truth(&condition.value)))
                } else {
                    TrackedValue::public(Value::Bool(truth(&condition.value)))
                };
                result.insert("failed".into(), outcome.clone())?;
                result.insert("failed_when_result".into(), outcome)?;
            }
            if let Some(expr) = task.get("changedWhen").filter(|v| !v.is_null()) {
                let condition = self.evaluate_scoped(expr, &current_scope, &iteration_layer)?;
                let outcome = if condition.is_secret() {
                    TrackedValue::secret(Value::Bool(truth(&condition.value)))
                } else {
                    TrackedValue::public(Value::Bool(truth(&condition.value)))
                };
                result.insert("changed".into(), outcome.clone())?;
                result.insert("changed_when_result".into(), outcome)?;
            }
            results.push(result);
        }
        let result = if looped {
            super::loop_result::aggregate(results, loop_source_sensitive)
        } else {
            results
                .into_iter()
                .next()
                .unwrap_or_else(|| TrackedValue::public(json!({"skipped":true,"changed":false})))
        };
        if let Some(name) = string(task, "register") {
            let mut results = self
                .scope
                .get("results")
                .unwrap_or_else(|| TrackedValue::public(json!({})));
            results.insert(name.to_owned(), result.clone())?;
            self.scope.insert("results".into(), results)?;
        }
        let is_failed = failed(&result.value);
        let ignored = is_failed && flag(task, "ignoreErrors");
        if flag(&result.value, "skipped") {
            self.recap.skipped += 1;
        } else if is_failed && !ignored {
            self.recap.failed += 1;
        } else {
            self.recap.ok += 1;
            if ignored {
                self.recap.ignored += 1;
            }
        }
        if flag(&result.value, "changed") {
            self.recap.changed += 1;
            if !is_failed {
                self.queue_handlers(task, &result);
            }
        }
        let visible = if layer.no_log || result.is_secret() {
            json!({"censored":true,"changed":flag(&result.value,"changed"),"failed":is_failed})
        } else {
            result.value
        };
        self.backend.event(
            &json!({"host":self.host,"task":string(task,"name").unwrap_or(""),"result":visible}),
        )?;
        Ok(!is_failed || ignored)
    }
}

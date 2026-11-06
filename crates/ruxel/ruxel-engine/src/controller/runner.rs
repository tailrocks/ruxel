use super::{
    Backend, Context, ContextLayer, Future, Map, Pin, Result, Runner, TrackedValue, Value, array,
    bail, failed, flag, json, object, result_scope, string, truth,
};

impl<B: Backend> Runner<'_, B> {
    pub(super) fn extend_vars(
        &mut self,
        vars: &mut std::collections::BTreeMap<String, TrackedValue>,
        incoming: &Map<String, Value>,
    ) -> Result<()> {
        let mut pending: Vec<_> = incoming.iter().collect();
        let mut previous_count = pending.len();
        while !pending.is_empty() {
            let mut unresolved = Vec::new();
            let mut last_error = None;
            for (key, value) in pending {
                let scope = self.scope(
                    &ContextLayer {
                        vars: vars.clone(),
                        ..ContextLayer::default()
                    },
                    None,
                );
                match self.backend.evaluate_tracked(value, &scope) {
                    Ok(value) => {
                        vars.insert(key.clone(), value);
                    }
                    Err(error) => {
                        unresolved.push((key, value));
                        last_error = Some(error);
                    }
                }
            }
            if unresolved.len() == incoming.len()
                || (!unresolved.is_empty() && unresolved.len() == previous_count)
            {
                return Err(last_error.context("variable evaluation failed")?);
            }
            previous_count = unresolved.len();
            pending = unresolved;
        }
        Ok(())
    }

    pub(super) fn layer(&mut self, task: &Value, parent: &ContextLayer) -> Result<ContextLayer> {
        let mut layer = parent.clone();
        self.extend_vars(&mut layer.vars, &object(task, "vars"))?;
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
        Ok(layer)
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
                if !self.task(task, layer).await? {
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
            let layer = self.layer(task, parent)?;
            if !self.handling
                && !self.options.tags.is_empty()
                && !layer
                    .tags
                    .iter()
                    .any(|tag| tag == "always" || self.options.tags.contains(tag))
            {
                return Ok(true);
            }
            if !array(task, "block").is_empty() {
                let scope = self.scope(&layer, None);
                if let Some(condition) = task.get("when").filter(|v| !v.is_null())
                    && !truth(&self.backend.evaluate_tracked(condition, &scope)?.value)
                {
                    self.recap.skipped += 1;
                    return Ok(true);
                }
                let mut success = self.tasks(array(task, "block"), &layer).await?;
                if !success && !array(task, "rescue").is_empty() {
                    success = self.tasks(array(task, "rescue"), &layer).await?;
                    if success {
                        self.recap.rescued += 1;
                        self.recap.failed = self.recap.failed.saturating_sub(1);
                    }
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
        let scope = self.scope(layer, None);
        let items = if let Some(expr) = task.get("loop").filter(|v| !v.is_null()) {
            let value = self.backend.evaluate_tracked(expr, &scope)?;
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
            let scope = self.scope(
                layer,
                looped.then_some((string(task, "loopVar").unwrap_or("item"), item)),
            );
            if let Some(condition) = task.get("when").filter(|v| !v.is_null())
                && !truth(&self.backend.evaluate_tracked(condition, &scope)?.value)
            {
                let mut skipped = TrackedValue::public(json!({"skipped":true,"changed":false}));
                if looped {
                    skipped.insert("item".into(), item.clone())?;
                }
                results.push(skipped);
                continue;
            }
            let mut result = self.execute(task, layer, &scope, item).await?;
            if !result.value.is_object() {
                bail!("operation result must be an object");
            }
            if looped {
                result.insert("item".into(), item.clone())?;
            }
            let mut current_scope = result_scope(&scope, task, &result)?;
            if let Some(expr) = task.get("failedWhen").filter(|v| !v.is_null()) {
                let condition = self.backend.evaluate_tracked(expr, &current_scope)?;
                result.insert(
                    "failed".into(),
                    if condition.is_secret() {
                        TrackedValue::secret(Value::Bool(truth(&condition.value)))
                    } else {
                        TrackedValue::public(Value::Bool(truth(&condition.value)))
                    },
                )?;
            }
            if let Some(expr) = task.get("changedWhen").filter(|v| !v.is_null()) {
                current_scope = result_scope(&scope, task, &result)?;
                let condition = self.backend.evaluate_tracked(expr, &current_scope)?;
                result.insert(
                    "changed".into(),
                    if condition.is_secret() {
                        TrackedValue::secret(Value::Bool(truth(&condition.value)))
                    } else {
                        TrackedValue::public(Value::Bool(truth(&condition.value)))
                    },
                )?;
            }
            results.push(result);
        }
        let result = if looped {
            let sensitive = results.iter().any(TrackedValue::is_secret);
            let result = json!({"changed":results.iter().any(|v| flag(&v.value,"changed")), "failed":results.iter().any(|v| failed(&v.value)), "skipped":results.iter().all(|v| flag(&v.value,"skipped")), "results":results.into_iter().map(|v| v.value).collect::<Vec<_>>()});
            if sensitive {
                TrackedValue::secret(result)
            } else {
                TrackedValue::public(result)
            }
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
                self.notified.extend(
                    array(task, "notify")
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
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

    pub(super) async fn run_handlers(&mut self, layer: &ContextLayer) -> Result<()> {
        let handlers = self.handlers.clone();
        for handler in &handlers {
            if string(handler, "name").is_some_and(|name| self.notified.remove(name)) {
                self.handling = true;
                let result = self.task(handler, layer).await;
                self.handling = false;
                if !result? {
                    break;
                }
            }
        }
        if !self.notified.is_empty() {
            bail!("unknown notified handlers: {:?}", self.notified);
        }
        Ok(())
    }
}

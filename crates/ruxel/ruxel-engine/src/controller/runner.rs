use super::{
    Backend, Context, ContextLayer, Future, Map, Pin, Result, Runner, Value, array, bail, failed,
    flag, json, object, string, truth,
};

impl<B: Backend> Runner<'_, B> {
    pub(super) fn extend_vars(
        &mut self,
        vars: &mut Map<String, Value>,
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
                match self.backend.evaluate(value, &scope) {
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
        item: Option<(&str, &Value)>,
    ) -> Map<String, Value> {
        let mut scope = self.scope.clone();
        let mut vars = layer.vars.clone();
        if let Some(facts) = self.scope.get("setFacts").and_then(Value::as_object) {
            vars.extend(facts.clone());
        }
        scope.insert("vars".into(), Value::Object(vars));
        if let Some((name, value)) = item {
            scope.insert(name.to_owned(), value.clone());
            scope.insert("item".into(), value.clone());
        }
        scope
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
            if !self.options.tags.is_empty()
                && !layer.tags.iter().any(|tag| self.options.tags.contains(tag))
            {
                return Ok(true);
            }
            if !array(task, "block").is_empty() {
                let scope = self.scope(&layer, None);
                if let Some(condition) = task.get("when").filter(|v| !v.is_null())
                    && !truth(&self.backend.evaluate(condition, &scope)?)
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
            self.backend
                .evaluate(expr, &scope)?
                .as_array()
                .cloned()
                .context("loop must evaluate to a list")?
        } else {
            vec![Value::Null]
        };
        let looped = task.get("loop").is_some_and(|v| !v.is_null());
        let mut results = Vec::new();
        for item in &items {
            let scope = self.scope(
                layer,
                looped.then_some((string(task, "loopVar").unwrap_or("item"), item)),
            );
            if let Some(condition) = task.get("when").filter(|v| !v.is_null())
                && !truth(&self.backend.evaluate(condition, &scope)?)
            {
                let mut skipped = json!({"skipped":true,"changed":false});
                if looped {
                    skipped["item"] = item.clone();
                }
                results.push(skipped);
                continue;
            }
            let mut result = self.execute(task, layer, &scope, item).await?;
            if !result.is_object() {
                bail!("operation result must be an object");
            }
            if looped {
                result["item"] = item.clone();
            }
            let mut result_scope = scope.clone();
            result_scope.insert("result".into(), result.clone());
            if let Some(expr) = task.get("failedWhen").filter(|v| !v.is_null()) {
                result["failed"] = Value::Bool(truth(&self.backend.evaluate(expr, &result_scope)?));
            }
            if let Some(expr) = task.get("changedWhen").filter(|v| !v.is_null()) {
                result["changed"] =
                    Value::Bool(truth(&self.backend.evaluate(expr, &result_scope)?));
            }
            results.push(result);
        }
        let result = if looped {
            json!({"changed":results.iter().any(|v| flag(v,"changed")), "failed":results.iter().any(failed), "skipped":results.iter().all(|v| flag(v,"skipped")), "results":results})
        } else {
            results
                .into_iter()
                .next()
                .unwrap_or_else(|| json!({"skipped":true,"changed":false}))
        };
        if let Some(name) = string(task, "register") {
            let results = self.scope.entry("results").or_insert_with(|| json!({}));
            results
                .as_object_mut()
                .context("results scope is not an object")?
                .insert(name.to_owned(), result.clone());
        }
        let is_failed = failed(&result);
        let ignored = is_failed && flag(task, "ignoreErrors");
        if flag(&result, "skipped") {
            self.recap.skipped += 1;
        } else if is_failed && !ignored {
            self.recap.failed += 1;
        } else {
            self.recap.ok += 1;
            if ignored {
                self.recap.ignored += 1;
            }
        }
        if flag(&result, "changed") {
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
        let visible = if layer.no_log {
            json!({"censored":true,"changed":flag(&result,"changed"),"failed":is_failed})
        } else {
            result
        };
        self.backend.event(
            &json!({"host":self.host,"task":string(task,"name").unwrap_or(""),"result":visible}),
        )?;
        Ok(!is_failed || ignored)
    }

    pub(super) async fn run_handlers(&mut self, layer: &ContextLayer) -> Result<()> {
        let handlers = self.handlers.clone();
        for handler in &handlers {
            if string(handler, "name").is_some_and(|name| self.notified.remove(name))
                && !self.task(handler, layer).await?
            {
                break;
            }
        }
        if !self.notified.is_empty() {
            bail!("unknown notified handlers: {:?}", self.notified);
        }
        Ok(())
    }
}

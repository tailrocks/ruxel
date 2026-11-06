//! Execution of an evaluated Pkl task graph.
//!
//! The backend owns transport and event presentation. The controller owns the
//! per-host scope and issues one iteration at a time so that later expressions
//! can observe prior results.

use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

/// Execution mode selected by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plan,
    Apply,
    Check,
    Diff,
}

/// An operation prepared for an agent, with its intended execution host.
#[derive(Debug, Clone)]
pub struct Request {
    pub host: String,
    pub task: v1::RenderedTask,
}

/// The controller delegates expression evaluation and I/O to the caller.
/// `evaluate` must recursively evaluate expression nodes within ordinary JSON.
#[expect(async_fn_in_trait, reason = "the backend is statically dispatched")]
pub trait Backend {
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value>;
    async fn execute(&mut self, request: Request) -> Result<Value>;
    fn event(&mut self, event: &Value) -> Result<()>;
}

/// Per-host execution totals.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recap {
    pub ok: u64,
    pub changed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub rescued: u64,
    pub ignored: u64,
}

/// Options for a run. `hosts` restricts the play's targets when nonempty.
#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Apply,
            hosts: Vec::new(),
            tags: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
struct ContextLayer {
    vars: Map<String, Value>,
    environment: Map<String, Value>,
    become_user: Option<String>,
    tags: Vec<String>,
}

struct Runner<'a, B> {
    backend: &'a mut B,
    options: &'a Options,
    host: String,
    scope: Map<String, Value>,
    handlers: Vec<Value>,
    notified: BTreeSet<String>,
    recap: Recap,
    next_id: u64,
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}
fn object(value: &Value, key: &str) -> Map<String, Value> {
    value
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}
fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64().is_some_and(|n| n != 0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
fn failed(value: &Value) -> bool {
    flag(value, "failed")
}
fn snake(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn group_hosts(
    name: &str,
    groups: &Map<String, Value>,
    visiting: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    if !visiting.insert(name.to_owned()) {
        bail!("inventory group cycle at {name:?}");
    }
    let group = groups
        .get(name)
        .with_context(|| format!("unknown inventory group {name:?}"))?;
    let mut hosts = BTreeSet::new();
    for host in array(group, "hosts") {
        hosts.insert(
            host.as_str()
                .context("group host must be a string")?
                .to_owned(),
        );
    }
    for child in array(group, "children") {
        hosts.extend(group_hosts(
            child.as_str().context("group child must be a string")?,
            groups,
            visiting,
        )?);
    }
    visiting.remove(name);
    Ok(hosts)
}

/// Expand explicit host and group targets and execute each play in order.
/// Facts are supplied by the caller, keyed by inventory host name.
pub async fn run<B: Backend>(
    config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    backend: &mut B,
) -> Result<BTreeMap<String, Recap>> {
    let inventory = config.get("inventory").context("missing inventory")?;
    let hosts = object(inventory, "hosts");
    let groups = object(inventory, "groups");
    let mut totals = BTreeMap::new();
    for play in array(config, "plays") {
        let mut targets = BTreeSet::new();
        for target in array(play, "targets") {
            let target = target.as_str().context("target must be a string")?;
            if target == "all" {
                targets.extend(hosts.keys().cloned());
            } else if hosts.contains_key(target) {
                targets.insert(target.to_owned());
            } else if groups.contains_key(target) {
                targets.extend(group_hosts(target, &groups, &mut BTreeSet::new())?);
            } else {
                bail!("unknown inventory target {target:?}");
            }
        }
        for host in targets {
            if !hosts.contains_key(&host) {
                bail!("unknown inventory host {host:?}");
            }
            if !options.hosts.is_empty() && !options.hosts.contains(&host) {
                continue;
            }
            let mut scope = Map::new();
            scope.insert("inventoryHost".into(), Value::String(host.clone()));
            scope.insert(
                "facts".into(),
                facts.get(&host).cloned().unwrap_or_else(|| json!({})),
            );
            let mut runner = Runner {
                backend,
                options,
                host: host.clone(),
                scope,
                handlers: array(play, "handlers").to_vec(),
                notified: BTreeSet::new(),
                recap: Recap::default(),
                next_id: 1,
            };
            let mut layer = ContextLayer::default();
            for (name, group) in &groups {
                if group_hosts(name, &groups, &mut BTreeSet::new())?.contains(&host) {
                    runner.extend_vars(&mut layer.vars, &object(group, "vars"))?;
                }
            }
            runner.extend_vars(&mut layer.vars, &object(&hosts[&host], "vars"))?;
            runner.extend_vars(&mut layer.vars, &object(play, "vars"))?;
            layer.environment = object(play, "environment");
            layer.become_user = string(play, "becomeUser").map(str::to_owned);
            let success = runner.tasks(array(play, "tasks"), &layer).await?;
            if success {
                runner.run_handlers(&layer).await?;
            }
            let recap = totals.entry(host).or_insert_with(Recap::default);
            recap.ok += runner.recap.ok;
            recap.changed += runner.recap.changed;
            recap.failed += runner.recap.failed;
            recap.skipped += runner.recap.skipped;
            recap.rescued += runner.recap.rescued;
            recap.ignored += runner.recap.ignored;
        }
    }
    Ok(totals)
}

impl<B: Backend> Runner<'_, B> {
    fn extend_vars(
        &mut self,
        vars: &mut Map<String, Value>,
        incoming: &Map<String, Value>,
    ) -> Result<()> {
        for (key, value) in incoming {
            let mut scope = self.scope.clone();
            scope.insert("vars".into(), Value::Object(vars.clone()));
            vars.insert(key.clone(), self.backend.evaluate(value, &scope)?);
        }
        Ok(())
    }

    fn layer(&mut self, task: &Value, parent: &ContextLayer) -> Result<ContextLayer> {
        let mut layer = parent.clone();
        self.extend_vars(&mut layer.vars, &object(task, "vars"))?;
        layer.environment.extend(object(task, "environment"));
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

    fn scope(&self, layer: &ContextLayer, item: Option<(&str, &Value)>) -> Map<String, Value> {
        let mut scope = self.scope.clone();
        scope.insert("vars".into(), Value::Object(layer.vars.clone()));
        if let Some((name, value)) = item {
            scope.insert(name.to_owned(), value.clone());
            scope.insert("item".into(), value.clone());
        }
        scope
    }

    fn tasks<'a>(
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

    fn task<'a>(
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
                if let Some(condition) = task.get("when").filter(|v| !v.is_null()) {
                    if !truth(&self.backend.evaluate(condition, &scope)?) {
                        self.recap.skipped += 1;
                        return Ok(true);
                    }
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

    async fn operation(&mut self, task: &Value, layer: &ContextLayer) -> Result<bool> {
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
            if let Some(condition) = task.get("when").filter(|v| !v.is_null()) {
                if !truth(&self.backend.evaluate(condition, &scope)?) {
                    results.push(json!({"skipped":true,"changed":false}));
                    continue;
                }
            }
            let mut result = self.execute(task, layer, &scope, item).await?;
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
        let visible = if flag(task, "noLog") {
            json!({"censored":true,"changed":flag(&result,"changed"),"failed":is_failed})
        } else {
            result
        };
        self.backend.event(
            &json!({"host":self.host,"task":string(task,"name").unwrap_or(""),"result":visible}),
        )?;
        Ok(!is_failed || ignored)
    }

    async fn execute(
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
                params.insert(snake(key), self.backend.evaluate(value, scope)?);
            }
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
        let destination = string(task, "delegateTo").unwrap_or(&self.host).to_owned();
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            host: destination,
            task: v1::RenderedTask {
                task_id: id,
                name: string(task, "name").unwrap_or(kind).to_owned(),
                module: kind.to_owned(),
                rendered: true,
                iterations: vec![v1::Iteration {
                    item_label: if item.is_null() {
                        String::new()
                    } else {
                        item.to_string()
                    },
                    params_json: serde_json::to_vec(&params)?,
                    free_form: String::new(),
                    ledger_key: String::new(),
                }],
                check_mode_override: task.get("checkMode").and_then(Value::as_bool) == Some(false),
                no_log: flag(task, "noLog"),
                become_user: layer.become_user.clone().unwrap_or_default(),
                environment,
                halt_on_failure: true,
                force_check_mode: self.options.mode == Mode::Check
                    || task.get("checkMode").and_then(Value::as_bool) == Some(true),
            },
        };
        if self.options.mode == Mode::Plan {
            let visible_params = if flag(task, "noLog") {
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
            let mut result_scope = scope.clone();
            result_scope.insert("result".into(), result.clone());
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

    async fn run_handlers(&mut self, layer: &ContextLayer) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        calls: Vec<Request>,
        events: Vec<Value>,
    }
    impl Backend for Fake {
        fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
            if value.get("kind").and_then(Value::as_str) == Some("ref") {
                let mut current = Value::Object(scope.clone());
                for key in value["path"].as_array().context("path")? {
                    current = current
                        .get(key.as_str().context("key")?)
                        .cloned()
                        .context("missing reference")?;
                }
                Ok(current)
            } else if value.get("kind").and_then(Value::as_str) == Some("literal") {
                Ok(value["value"].clone())
            } else {
                Ok(value.clone())
            }
        }
        async fn execute(&mut self, request: Request) -> Result<Value> {
            self.calls.push(request);
            Ok(json!({"changed":true}))
        }
        fn event(&mut self, event: &Value) -> Result<()> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn nested_targets_loop_and_registered_result() -> Result<()> {
        let config = json!({"inventory":{"hosts":{"host":{}},"groups":{"outer":{"children":["inner"]},"inner":{"hosts":["host"]}}},"plays":[{"targets":["outer"],"tasks":[{"name":"loop","loop":["a","b"],"operation":{"kind":"command","cmd":{"kind":"ref","path":["item"]}},"register":"first"},{"name":"next","when":{"kind":"ref","path":["results","first","changed"]},"operation":{"kind":"command","cmd":"next"}}]}]});
        let mut backend = Fake::default();
        let recaps = run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
        assert_eq!(backend.calls.len(), 3);
        assert_eq!(recaps["host"].changed, 2);
        assert_eq!(
            serde_json::from_slice::<Value>(&backend.calls[0].task.iterations[0].params_json)?["cmd"],
            "a"
        );
        Ok(())
    }

    #[test]
    fn group_cycles_are_rejected() {
        let groups = json!({"a":{"children":["b"]},"b":{"children":["a"]}});
        assert!(group_hosts("a", groups.as_object().unwrap(), &mut BTreeSet::new()).is_err());
    }
}

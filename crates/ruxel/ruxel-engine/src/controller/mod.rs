use crate::expression::provenance::TrackedValue;
use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

mod standard_expression;
mod types;
mod validation;
mod variables;
pub use standard_expression::evaluate_standard_layered;
pub use types::{Mode, Options, Recap, Request};

#[expect(async_fn_in_trait, reason = "the backend is statically dispatched")]
pub trait Backend {
    fn conservative_sensitive(&self) -> bool {
        false
    }
    /// # Errors
    /// Returns an error if evaluation fails.
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value>;
    /// # Errors
    /// Returns an error if evaluation fails.
    fn evaluate_tracked(
        &mut self,
        value: &Value,
        scope: &crate::expression::provenance::TrackedValue,
    ) -> Result<crate::expression::provenance::TrackedValue> {
        use crate::expression::provenance::TrackedValue;
        let sensitive = scope.is_secret();
        let scope = scope.value.as_object().context("scope must be an object")?;
        let value = self.evaluate(value, scope).map_err(|error| {
            if sensitive {
                anyhow::anyhow!("sensitive evaluation failed")
            } else {
                error
            }
        })?;
        Ok(if sensitive {
            TrackedValue::secret(value)
        } else {
            TrackedValue::public(value)
        })
    }
    /// Evaluates only accessed bindings, preserving shadowing, runtime scope and provenance.
    /// Implementations must not materialize unused layers or unselected branches.
    ///
    /// # Errors
    /// Returns an error if a required expression or binding cannot be evaluated.
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)>;
    /// # Errors
    /// Returns an error if the source cannot be prepared.
    fn prepare_operation(
        &mut self,
        _kind: &str,
        _params: &mut Map<String, Value>,
        _scope: &Map<String, Value>,
    ) -> Result<()> {
        Ok(())
    }
    async fn prepare_operation_tracked(
        &mut self,
        kind: &str,
        params: &mut Map<String, Value>,
        scope: &Map<String, Value>,
        sensitive: bool,
    ) -> Result<bool> {
        self.prepare_operation(kind, params, scope)
            .map_err(|error| {
                if sensitive {
                    anyhow::anyhow!("sensitive operation preparation failed")
                } else {
                    error
                }
            })?;
        Ok(sensitive)
    }
    async fn execute(&mut self, request: Request) -> Result<Value>;
    async fn execute_tracked(
        &mut self,
        mut request: Request,
        sensitive: bool,
    ) -> Result<TrackedValue> {
        request.task.no_log |= sensitive;
        let result = self.execute(request).await.map_err(|error| {
            if sensitive {
                anyhow::anyhow!("sensitive operation failed")
            } else {
                error
            }
        })?;
        Ok(if sensitive {
            TrackedValue::secret(result)
        } else {
            TrackedValue::public(result)
        })
    }
    /// # Errors
    /// Returns an error if the event cannot be written.
    fn event(&mut self, event: &Value) -> Result<()>;
}

#[derive(Clone, Default)]
struct ContextLayer {
    vars: BTreeMap<String, TrackedValue>,
    definitions: Vec<Map<String, Value>>,
    environment: Map<String, Value>,
    become_user: Option<String>,
    delegate_to: Option<Value>,
    check_mode: Option<bool>,
    tags: Vec<String>,
    no_log: bool,
    control_sensitive: bool,
}

struct Runner<'a, B> {
    backend: &'a mut B,
    options: &'a Options,
    host: String,
    scope: TrackedValue,
    handlers: Vec<Value>,
    notified: BTreeMap<String, bool>,
    handling: bool,
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

fn result_scope(scope: &TrackedValue, task: &Value, result: &TrackedValue) -> Result<TrackedValue> {
    let mut next = scope.clone();
    next.insert("result".into(), result.clone())?;
    if let Some(name) = string(task, "register") {
        let mut results = next
            .get("results")
            .unwrap_or_else(|| TrackedValue::public(json!({})));
        results.insert(name.to_owned(), result.clone())?;
        next.insert("results".into(), results)?;
    }
    Ok(next)
}

/// # Errors
/// Returns an error if inventory, expression evaluation, or execution fails.
#[expect(
    clippy::too_many_lines,
    reason = "host execution preserves play order and shared scope"
)]
pub async fn run<B: Backend>(
    config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    backend: &mut B,
    set_facts: &mut BTreeMap<String, TrackedValue>,
) -> Result<BTreeMap<String, Recap>> {
    validation::validate(config)?;
    selected_hosts(config, options)?;
    let inventory = config.get("inventory").context("missing inventory")?;
    let hosts = object(inventory, "hosts");
    let groups = object(inventory, "groups");
    let mut totals = BTreeMap::new();
    for play in array(config, "plays") {
        let selectors = array(play, "targets")
            .iter()
            .map(|value| value.as_str().context("target must be a string"))
            .collect::<Result<Vec<_>>>()?;
        let targets = expand_selectors(&selectors, &hosts, &groups)?;
        let filtered: BTreeSet<_> = selected_hosts(config, options)?.into_iter().collect();
        for host in targets {
            if !hosts.contains_key(&host) {
                bail!("unknown inventory host {host:?}");
            }
            if !filtered.contains(&host) {
                continue;
            }
            if options.mode == Mode::Plan {
                let inherited: Vec<String> = array(play, "tags")
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let mut planned = plan::plan_tasks(
                    backend,
                    &host,
                    array(play, "preTasks"),
                    "preTask",
                    &options.tags,
                    &inherited,
                )?;
                planned += plan::plan_tasks(
                    backend,
                    &host,
                    array(play, "tasks"),
                    "task",
                    &options.tags,
                    &inherited,
                )?;
                plan::plan_handlers(backend, &host, array(play, "handlers"))?;
                totals.entry(host).or_insert_with(Recap::default).planned += planned;
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
                scope: TrackedValue::public(Value::Object(scope)),
                handlers: array(play, "handlers").to_vec(),
                notified: BTreeMap::new(),
                handling: false,
                recap: Recap::default(),
                next_id: 1,
            };
            if let Some(persisted) = set_facts.get(&host) {
                runner.scope.insert("setFacts".into(), persisted.clone())?;
            }
            let mut layer = ContextLayer {
                control_sensitive: runner.backend.conservative_sensitive(),
                ..ContextLayer::default()
            };
            for name in ordered_groups(&groups)? {
                if group_hosts(&name, &groups, &mut BTreeSet::new())?.contains(&host) {
                    layer.definitions.push(object(&groups[&name], "vars"));
                }
            }
            layer.definitions.push(object(&hosts[&host], "vars"));
            layer.definitions.push(object(play, "vars"));
            layer.tags = array(play, "tags")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            layer.environment = object(play, "environment");
            layer.become_user = string(play, "becomeUser").map(str::to_owned);
            let execution: Result<()> = async {
                let success = runner.tasks(array(play, "preTasks"), &layer).await?
                    && runner.tasks(array(play, "tasks"), &layer).await?;
                if success {
                    runner.run_handlers(&layer).await?;
                }
                Ok(())
            }
            .await;
            if let Some(persisted) = runner.scope.get("setFacts") {
                set_facts.insert(host.clone(), persisted);
            }
            let recap = totals.entry(host).or_insert_with(Recap::default);
            recap.ok += runner.recap.ok;
            recap.changed += runner.recap.changed;
            recap.failed += runner.recap.failed;
            recap.skipped += runner.recap.skipped;
            recap.rescued += runner.recap.rescued;
            recap.ignored += runner.recap.ignored;
            if let Err(error) = execution {
                let error = if layer.control_sensitive || runner.backend.conservative_sensitive() {
                    anyhow::anyhow!("sensitive execution failed")
                } else {
                    error
                };
                return Err(RunFailure::new(error, totals).into());
            }
        }
    }
    Ok(totals)
}

mod assertion;
mod failure;
mod handlers;
pub(crate) use failure::RunFailure;
mod ledger;
mod loop_result;
mod selection;
pub use ledger::ledger_key;
pub use selection::selected_hosts;
use selection::{expand_selectors, group_hosts, ordered_groups};
mod local;
mod loop_vars;
mod operation;
mod plan;
mod retry;
mod runner;
mod tag_selection;

#[cfg(test)]
mod tests;

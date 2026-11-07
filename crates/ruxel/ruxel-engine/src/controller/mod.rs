//! Execution of an evaluated Pkl task graph.
//!
//! The backend owns transport and event presentation. The controller owns the
//! per-host scope and issues one iteration at a time so that later expressions
//! can observe prior results.

use crate::expression::provenance::TrackedValue;
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
    /// Evaluate an expression in the current scope.
    ///
    /// # Errors
    /// Returns an error if evaluation fails.
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value>;
    /// Evaluate while retaining secret provenance from the scope.
    ///
    /// Backends that do not implement provenance must conservatively mark the
    /// result sensitive whenever the supplied scope contains a secret.
    ///
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
    /// Evaluate a task expression while resolving task variables as needed.
    /// The default conservatively resolves referenced variables; backends may
    /// override it to preserve short-circuit evaluation within expressions.
    ///
    /// # Errors
    /// Returns an error if an accessed variable or expression cannot be evaluated.
    fn evaluate_task_expression(
        &mut self,
        value: &Value,
        scope: &crate::expression::provenance::TrackedValue,
        vars: &Map<String, Value>,
    ) -> Result<(
        crate::expression::provenance::TrackedValue,
        std::collections::BTreeMap<String, crate::expression::provenance::TrackedValue>,
    )> {
        fallback::evaluate(self, value, scope, vars)
    }
    /// Prepare source-backed operations before evaluating their parameter expressions.
    ///
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
    /// Prepare a source and report whether it is sensitive.
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
    /// Execute a request and retain sensitivity of its inputs and result.
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
    /// Emit a structured execution event.
    ///
    /// # Errors
    /// Returns an error if the event cannot be written.
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
    pub planned: u64,
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
    vars: BTreeMap<String, TrackedValue>,
    environment: Map<String, Value>,
    become_user: Option<String>,
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

/// Expand explicit host and group targets and execute each play in order.
/// Facts are supplied by the caller, keyed by inventory host name. Set facts
/// persist across plays through `set_facts`, including their provenance.
///
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
    // Validate every target before the first backend call.
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
            let mut layer = ContextLayer::default();
            for name in ordered_groups(&groups)? {
                if group_hosts(&name, &groups, &mut BTreeSet::new())?.contains(&host) {
                    runner.extend_vars(&mut layer.vars, &object(&groups[&name], "vars"))?;
                }
            }
            runner.extend_vars(&mut layer.vars, &object(&hosts[&host], "vars"))?;
            runner.extend_vars(&mut layer.vars, &object(play, "vars"))?;
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
mod fallback;
mod local;
mod loop_vars;
mod operation;
mod plan;
mod retry;
mod runner;
mod tag_selection;

#[cfg(test)]
mod tests;

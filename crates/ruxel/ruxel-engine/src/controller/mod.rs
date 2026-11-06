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
    /// Evaluate an expression in the current scope.
    ///
    /// # Errors
    /// Returns an error if evaluation fails.
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value>;
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
    async fn execute(&mut self, request: Request) -> Result<Value>;
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
    no_log: bool,
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

fn result_scope(
    scope: &Map<String, Value>,
    task: &Value,
    result: &Value,
) -> Result<Map<String, Value>> {
    let mut next = scope.clone();
    next.insert("result".into(), result.clone());
    if let Some(name) = string(task, "register") {
        next.entry("results")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("results scope is not an object")?
            .insert(name.to_owned(), result.clone());
    }
    Ok(next)
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

/// Resolve and validate play targets without contacting any host.
///
/// # Errors
/// Returns an error for unknown targets or invalid inventory groups.
pub fn selected_hosts(config: &Value, options: &Options) -> Result<Vec<String>> {
    let inventory = config.get("inventory").context("missing inventory")?;
    let hosts = object(inventory, "hosts");
    let groups = object(inventory, "groups");
    for name in groups.keys() {
        for host in group_hosts(name, &groups, &mut BTreeSet::new())? {
            if !hosts.contains_key(&host) {
                bail!("unknown inventory host {host:?}");
            }
        }
    }
    for host in &options.hosts {
        if !hosts.contains_key(host) {
            bail!("unknown host filter {host:?}");
        }
    }
    let mut selected = BTreeSet::new();
    for play in array(config, "plays") {
        for value in array(play, "targets") {
            let target = value.as_str().context("target must be a string")?;
            if target == "all" {
                selected.extend(hosts.keys().cloned());
            } else if hosts.contains_key(target) {
                selected.insert(target.to_owned());
            } else if groups.contains_key(target) {
                selected.extend(group_hosts(target, &groups, &mut BTreeSet::new())?);
            } else {
                bail!("unknown inventory target {target:?}");
            }
        }
    }
    for host in &selected {
        if !hosts.contains_key(host) {
            bail!("unknown inventory host {host:?}");
        }
    }
    if !options.hosts.is_empty() {
        selected.retain(|host| options.hosts.contains(host));
    }
    Ok(selected.into_iter().collect())
}

fn visible_plan(task: &Value) -> Value {
    if flag(task, "noLog") {
        return json!({"name":string(task,"name"),"noLog":true,"censored":true});
    }
    let mut visible = task.clone();
    for key in ["block", "rescue", "always"] {
        if let Some(children) = visible.get_mut(key).and_then(Value::as_array_mut) {
            for child in children {
                *child = visible_plan(child);
            }
        }
    }
    visible
}

fn plan_tasks<B: Backend>(backend: &mut B, host: &str, tasks: &[Value], phase: &str) -> Result<()> {
    for task in tasks {
        backend
            .event(&json!({"host":host,"phase":phase,"planned":true,"task":visible_plan(task)}))?;
    }
    Ok(())
}

/// Expand explicit host and group targets and execute each play in order.
/// Facts are supplied by the caller, keyed by inventory host name.
///
/// # Errors
/// Returns an error if inventory, expression evaluation, or execution fails.
pub async fn run<B: Backend>(
    config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    backend: &mut B,
) -> Result<BTreeMap<String, Recap>> {
    // Validate every target before the first backend call.
    selected_hosts(config, options)?;
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
            if options.mode == Mode::Plan {
                plan_tasks(backend, &host, array(play, "preTasks"), "preTask")?;
                plan_tasks(backend, &host, array(play, "tasks"), "task")?;
                plan_tasks(backend, &host, array(play, "handlers"), "handler")?;
                totals.entry(host).or_insert_with(Recap::default);
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
            let success = runner.tasks(array(play, "preTasks"), &layer).await?
                && runner.tasks(array(play, "tasks"), &layer).await?;
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

mod ledger;
pub use ledger::ledger_key;
mod operation;
mod runner;

#[cfg(test)]
mod tests;

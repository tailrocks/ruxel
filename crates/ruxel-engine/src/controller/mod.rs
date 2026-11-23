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
    /// Monotonic connection-failure count, independent of redacted diagnostics.
    fn connection_failures(&self) -> u64 {
        0
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
    /// The keyed fingerprint of a secret-bearing task's content, built
    /// from the selectors alone. The controller asks for it before the
    /// operation's expressions are evaluated: the evaluated parameters
    /// carry the placeholders the selectors resolve to, not the selectors
    /// themselves.
    fn content_fingerprint(&self, _params: &Value) -> Option<String> {
        None
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
    delegate_allow: Vec<String>,
    delegate_localhost: bool,
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
    /// Tag selection omitted a task this workflow keeps: the run is a
    /// filtered subset, so its end must expire nothing.
    filtered: bool,
    /// Every session this run sent a request to: an incomplete origin
    /// marks its destinations incomplete too, so a delegated host never
    /// sends an unfiltered Done off its unrelated origin recap.
    delegated: BTreeSet<String>,
    /// The full configuration's static sysctl declarations that run on
    /// this host: every unconditional top-level sysctl task's entry in
    /// every play the host runs, computed before the first task
    /// executes, so each frame carries the declaration list the agent
    /// merges before running its task.
    plan_sysctl: Vec<v1::SysctlDeclaration>,
    /// The full configuration the scheduler split the play out of: a
    /// delegated request is scoped to its destination's declarations,
    /// not the source host's, so that set is recomputed from this
    /// at delegation time.
    full_config: &'a Value,
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

mod assertion;
mod delegation;
pub(crate) use delegation::mark_delegation_destinations;
mod failure;
mod handlers;
pub(crate) use failure::RunFailure;
mod ledger;
mod loop_result;
mod selection;
mod static_sysctl;
pub use ledger::ledger_key;
pub use run::{run, run_with_full_config};
pub use selection::selected_hosts;
mod local;
mod loop_vars;
mod operation;
mod plan;
mod retry;
mod run;
mod runner;
mod tag_selection;

#[cfg(test)]
mod tests;

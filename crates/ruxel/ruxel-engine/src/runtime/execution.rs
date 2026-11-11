//! The execution backend: how the controller evaluates, redacts, sends
//! tasks to hosts, and receives their results.

use super::{Execution, Request, Scope, copy_sources, lazy_vars, output, templates};
use crate::controller::{Backend, ledger_key};
use crate::expression::provenance::TrackedValue;
use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

impl Backend for Execution {
    fn connection_failures(&self) -> u64 {
        self.connection_failures
    }
    fn conservative_sensitive(&self) -> bool {
        self.conservative_sensitive
            .load(std::sync::atomic::Ordering::Acquire)
    }
    fn prepare_operation(
        &mut self,
        kind: &str,
        params: &mut Map<String, Value>,
        scope: &Map<String, Value>,
    ) -> Result<()> {
        templates::prepare(self, kind, params, scope)
    }
    fn evaluate(&mut self, value: &Value, map: &Map<String, Value>) -> Result<Value> {
        self.prefetch_lookups(value)?;
        let scope = Scope {
            facts: map.get("facts").cloned().unwrap_or(Value::Null),
            results: map.get("results").cloned().unwrap_or(Value::Null),
            variables: map.get("vars").cloned().unwrap_or(Value::Null),
            item: map.get("item").cloned().unwrap_or(Value::Null),
            extra: map.clone(),
        };
        self.evaluate_value(value, &scope)
    }
    /// The fingerprint of a secret-bearing content: the controller asks
    /// for it before the content is evaluated, so the selectors are still
    /// there.
    fn content_fingerprint(&self, params: &Value) -> Option<String> {
        self.lookups.content_fingerprint(params)
    }
    fn evaluate_tracked(&mut self, value: &Value, scope: &TrackedValue) -> Result<TrackedValue> {
        self.prefetch_lookups(value)?;
        self.evaluate_value_tracked(value, scope)
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        self.prefetch_lookups(value)?;
        lazy_vars::evaluate_layers(self, value, scope, layers)
    }
    async fn prepare_operation_tracked(
        &mut self,
        kind: &str,
        params: &mut Map<String, Value>,
        scope: &Map<String, Value>,
        mut sensitive: bool,
    ) -> Result<bool> {
        sensitive |= self.conservative_sensitive();
        if kind != "template" || params.get("content").is_some_and(|v| !v.is_null()) {
            self.prepare_operation(kind, params, scope)
                .map_err(|error| {
                    if sensitive {
                        anyhow::anyhow!("sensitive operation preparation failed")
                    } else {
                        error
                    }
                })?;
            return Ok(sensitive);
        }
        let gate = self.source_gate.clone();
        let _guard = gate.write().await;
        sensitive |= self.conservative_sensitive();
        let trackers: Vec<_> = self.resources.lock().await.values().cloned().collect();
        for tracker in trackers {
            sensitive |= tracker.lock().await.has_sensitive_effects();
        }
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
    async fn execute_tracked(
        &mut self,
        mut request: Request,
        sensitive: bool,
    ) -> Result<TrackedValue> {
        if !self.connections.lock().await.contains_key(&request.host) {
            self.connect(&request.host).await.map_err(|error| {
                self.connection_failures += 1;
                if sensitive || self.conservative_sensitive() {
                    anyhow::anyhow!("sensitive connection failed")
                } else {
                    error
                }
            })?;
        }
        let gate = self.source_gate.clone();
        let _guard = gate.read().await;
        let effects = self.effect_gate.clone();
        let _effect_read;
        let _effect_write;
        if sensitive && !self.conservative_sensitive() {
            _effect_read = None;
            _effect_write = Some(effects.write().await);
        } else {
            _effect_read = Some(effects.read().await);
            _effect_write = None;
        }
        let sensitive = sensitive || self.conservative_sensitive();
        let host = request.host.clone();
        let tracker = {
            let mut resources = self.resources.lock().await;
            resources.entry(host).or_default().clone()
        };
        let mut resources = tracker.lock().await;
        let sensitive = resources.observe(&request, sensitive)?;
        if sensitive {
            self.conservative_sensitive
                .store(true, std::sync::atomic::Ordering::Release);
        }
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
    async fn execute(&mut self, mut request: Request) -> Result<Value> {
        copy_sources::prepare(&mut request.task, &self.copy_root)?;
        for index in 0..request.task.iterations.len() {
            let key = ledger_key(&request.task, &request.task.iterations[index])?;
            request.task.iterations[index].ledger_key = key;
        }
        let connection = self
            .connections
            .lock()
            .await
            .get(&request.host)
            .cloned()
            .context("execution host is not connected")?;
        let mut connection = connection.lock().await;
        let id = request.task.task_id;
        let module = request.task.module.clone();
        let no_log = request.task.no_log;
        let fingerprint = request.content_fingerprint.clone();
        let params = request
            .task
            .iterations
            .first()
            .and_then(|iteration| serde_json::from_slice::<Value>(&iteration.params_json).ok());
        let plan = copy_sources::plan(request.task)?;
        connection.send(&plan).await.inspect_err(|_| {
            self.connection_failures += 1;
        })?;
        loop {
            let event = connection
                .next()
                .await
                .and_then(|event| event.context("agent closed before task result"))
                .inspect_err(|_| {
                    self.connection_failures += 1;
                })?;
            match event.msg {
                Some(v1::event::Msg::TaskResult(result)) if result.task_id == id => {
                    let mut value: Value = serde_json::from_slice(&result.result_json)
                        .context("invalid agent result")?;
                    // A FetchSecret result never carries the content: the
                    // controller takes the slurped bytes, compares them with
                    // the stored value, and writes fnox only when they
                    // differ, with the value on stdin. The comparison and
                    // the store happen here, on the controller, in apply
                    // only; in check and diff the task never reaches the
                    // host (the controller answers known-after-apply).
                    //
                    // A failed task is a failed task, whatever its module:
                    // its result carries no content, and the run reports
                    // the failure the way every other failed task is
                    // reported — the recap counts it, `ignoreErrors`
                    // ignores it, a retry retries it and a rescue rescues
                    // it. Only a result that succeeded carries content.
                    if module == "fetchSecret" && result.status != "failed" {
                        return self.receive_fetch_secret(&mut value, params.as_ref(), &result);
                    }
                    // A result whose task is censored — no-log, sensitive,
                    // or the run turned conservative — never carries an
                    // agent diff: the receive below strips the fields the
                    // agent shipped anyway and drops the protocol's own, so
                    // only the controller's sentence can be in the value.
                    let censored = no_log || self.conservative_sensitive();
                    Self::receive_task_result(
                        &module,
                        fingerprint.as_deref(),
                        censored,
                        &mut value,
                        &result,
                    );
                    return Ok(value);
                }
                Some(v1::event::Msg::Crash(_)) => {
                    self.connection_failures += 1;
                    bail!("agent crashed");
                }
                Some(v1::event::Msg::PauseRequest(_)) => bail!("unexpected agent pause"),
                _ => {}
            }
        }
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        let event = if self.conservative_sensitive() {
            output::sensitive_event(event)
        } else {
            self.lookups.redact(event)
        };
        self.output.event(&event)
    }
}

impl Execution {
    /// The apply-time `FetchSecret` receive: decode the slurped bytes, store
    /// them in fnox when they differ from the stored value, and strip the
    /// content from the result — the value never reaches output or results.
    /// The decoded value is registered with the redaction registry the
    /// moment it arrives, so a later task that echoes it is censored in
    /// every derived form. Under the synthetic resolver nothing is ever
    /// written: the fetch reports known-after-apply and unchanged.
    fn receive_fetch_secret(
        &self,
        value: &mut Value,
        params: Option<&Value>,
        result: &v1::TaskResult,
    ) -> Result<Value> {
        let encoded = value["content"]
            .as_str()
            .context("fetchSecret result carries no content")?;
        let key = params
            .and_then(|params| params.get("key").and_then(Value::as_str).map(str::to_owned))
            .context("fetchSecret has no key")?;
        let decoded = base64_decode(encoded)?;
        let content = String::from_utf8(decoded)
            .map_err(|_| anyhow::anyhow!("fetched content is not text"))?;
        // The content never leaves this function: strip it from the result
        // before anything can carry it into results, stdout or the event
        // log, and register the value with the redaction registry first.
        // The fingerprint and the diff of a fetch are the controller's too:
        // the agent's result is a slurp that carries only the content, so
        // fingerprint or diff fields it ships anyway are agent material,
        // and they are stripped here rather than trusted — the controller
        // builds both itself below.
        let object = value
            .as_object_mut()
            .context("fetchSecret result must be an object")?;
        object.remove("content");
        object.remove("fingerprint");
        object.remove("diff");
        self.lookups.record(&content);
        let synthetic = self.lookups.resolver.is_synthetic();
        let (changed, fingerprint) = self
            .lookups
            .resolver
            .fetch_secret(&key, &content)
            .map_err(anyhow::Error::msg)?;
        if changed {
            value["diff"] = json!(format!("stored in fnox (fingerprint {fingerprint})"));
        }
        if synthetic {
            // The synthetic resolver writes nothing: the fetch reports
            // known-after-apply and unchanged, the same shape check and
            // diff answer with.
            value["knownAfterApply"] = Value::Bool(true);
        }
        value["fingerprint"] = json!(fingerprint);
        value["changed"] = Value::Bool(changed);
        value["failed"] = Value::Bool(result.status == "failed");
        Ok(value.clone())
    }

    /// A secret-bearing file in check and diff shows a keyed, truncated
    /// fingerprint of the placeholder its lookups resolve to, never text and
    /// never an unsalted hash of a value. The agent ships no diff for a
    /// no-log task; the sentence is built here, from the selectors alone.
    /// The fingerprint reaches this far from the controller, which read it
    /// from the operation's parameters before they were evaluated.
    fn receive_task_result(
        module: &str,
        fingerprint: Option<&str>,
        censored: bool,
        value: &mut Value,
        result: &v1::TaskResult,
    ) {
        value["changed"] = Value::Bool(result.changed);
        value["failed"] = Value::Bool(result.status == "failed");
        // The fingerprint of a result is controller-built — the agent ships
        // none — so an agent-supplied `fingerprint` field is stripped here
        // and only the controller's own, set below, ever reaches output.
        // The diff is the same, for a task whose result is censored: the
        // agent ships none for one, so a `diff` field it ships
        // anyway is agent material, and it is dropped whole — a sentence
        // that carries the controller's shape proves nothing about who
        // built it, and eight lowercase hex characters hold four bytes of
        // anything, which is the whole of the channel when shape is the
        // test. The protocol's diff field is dropped for every task
        // alike: the agent's result already carries its own display
        // material, and a hostile one carries whatever it likes.
        if let Some(object) = value.as_object_mut() {
            object.remove("fingerprint");
            if censored {
                object.remove("diff");
            }
        }
        if module == "copy"
            && result.changed
            && let Some(fingerprint) = fingerprint
        {
            value["diff"] = json!(format!("content changed (fingerprint {fingerprint})"));
            value["fingerprint"] = json!(fingerprint);
        }
    }
}

/// Decodes the base64 an agent slurp result carries.
pub(super) fn base64_decode(text: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .context("invalid base64 in agent result")
}

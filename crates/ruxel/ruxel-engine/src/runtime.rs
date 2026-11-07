//! Connect the typed task controller to actual agent processes.
use crate::{
    controller::{self, Backend, Mode, Options, Recap, Request},
    expression::{Scope, provenance::TrackedValue},
    secrets::{ProcessBackend, Resolver},
    transport,
};
use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

mod options;
pub use options::RuntimeOptions;

mod connection;
use connection::Connection;

mod output;
use output::Output;
pub use output::OutputFormat;

mod lookups;
use lookups::Lookups;
mod lazy_vars;
mod resources;
use resources::Resources;

#[derive(Clone)]
struct Execution {
    inventory: Map<String, Value>,
    runtime: RuntimeOptions,
    mode: Mode,
    connections: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Connection>>>>>,
    lookups: Arc<Lookups>,
    output: Arc<Output>,
    resources: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Resources>>>>>,
    source_gate: Arc<tokio::sync::RwLock<()>>,
}
impl Execution {
    async fn connect(&self, host: &str) -> Result<v1::Facts> {
        let mut connections = self.connections.lock().await;
        if connections.contains_key(host) {
            return Ok(v1::Facts::default());
        }
        let spec = self.inventory.get(host);
        let controller_local = host == "localhost" && spec.is_none();
        anyhow::ensure!(
            spec.is_some() || controller_local,
            "selected host is missing"
        );
        let check = matches!(self.mode, Mode::Check | Mode::Diff);
        let diff = self.mode == Mode::Diff;
        let (connection, gathered) = if self.runtime.local_state.is_some() || controller_local {
            let state = self
                .runtime
                .local_state
                .as_ref()
                .or(self.runtime.controller_state.as_ref())
                .context("localhost delegation requires --controller-state-dir")?;
            let binary = if self.runtime.local_state.is_some() {
                &self.runtime.agent
            } else {
                self.runtime
                    .local_agent
                    .as_ref()
                    .context("localhost delegation requires --local-agent")?
            };
            // Host names are inventory keys, never accepted as filesystem paths.
            let path = state.join(blake3::hash(host.as_bytes()).to_hex().as_str());
            let (c, f) = transport::LocalConnection::connect(
                binary,
                &path,
                "pkl-local",
                check,
                diff,
                self.runtime.no_cache,
            )
            .await?;
            (Connection::Local(Box::new(c)), f)
        } else {
            let spec = spec.context("selected host is missing")?;
            let address = spec
                .get("address")
                .and_then(Value::as_str)
                .context("host address is missing")?;
            let destination = match spec.get("user").and_then(Value::as_str) {
                Some(user) => format!("{user}@{address}"),
                None => address.to_owned(),
            };
            let port = spec
                .get("port")
                .and_then(Value::as_u64)
                .map(u16::try_from)
                .transpose()?;
            let transport_options = transport::ConnectOptions {
                diff_mode: diff,
                no_cache: self.runtime.no_cache,
                port,
                ..self.runtime.connection.clone()
            };
            let (c, f) = transport::connect_with(
                &destination,
                &self.runtime.agent,
                "pkl",
                check,
                &transport_options,
            )
            .await?;
            (Connection::Remote(Box::new(c)), f.facts)
        };
        connections.insert(
            host.to_owned(),
            Arc::new(tokio::sync::Mutex::new(connection)),
        );
        Ok(gathered)
    }
}
mod evaluation;
mod templates;

impl Backend for Execution {
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
    fn evaluate_tracked(&mut self, value: &Value, scope: &TrackedValue) -> Result<TrackedValue> {
        self.prefetch_lookups(value)?;
        self.evaluate_value_tracked(value, scope)
    }
    fn evaluate_task_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        vars: &Map<String, Value>,
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        lazy_vars::evaluate(self, value, scope, vars)
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
        let gate = self.source_gate.clone();
        let _guard = gate.read().await;
        let host = request.host.clone();
        let tracker = {
            let mut resources = self.resources.lock().await;
            resources.entry(host).or_default().clone()
        };
        // Serialize requests to the same destination so a read cannot race a
        // secret write before its provenance is recorded.
        let mut resources = tracker.lock().await;
        let sensitive = resources.observe(&request, sensitive)?;
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
        if request.task.module == "copy" {
            for iteration in &mut request.task.iterations {
                let mut params: Map<String, Value> =
                    serde_json::from_slice(&iteration.params_json)?;
                let source = params.remove("src").filter(|v| !v.is_null());
                if let Some(src) =
                    source.filter(|_| params.get("content").is_none_or(Value::is_null))
                {
                    let source = self
                        .runtime
                        .source_dir
                        .join(src.as_str().context("source path must be text")?);
                    iteration.source_content =
                        Some(std::fs::read(source).context("read controller source file")?);
                    params.remove("content");
                }
                iteration.params_json = serde_json::to_vec(&params)?;
            }
        }
        for index in 0..request.task.iterations.len() {
            let key = controller::ledger_key(&request.task, &request.task.iterations[index])?;
            request.task.iterations[index].ledger_key = key;
        }
        if !self.connections.lock().await.contains_key(&request.host) {
            self.connect(&request.host).await?;
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
        connection
            .send(&v1::Envelope {
                msg: Some(v1::envelope::Msg::Plan(v1::Plan {
                    tasks: vec![request.task],
                    blobs_referenced: vec![],
                })),
            })
            .await?;
        loop {
            let event = connection
                .next()
                .await?
                .context("agent closed before task result")?;
            match event.msg {
                Some(v1::event::Msg::TaskResult(result)) if result.task_id == id => {
                    let mut value: Value = serde_json::from_slice(&result.result_json)
                        .context("invalid agent result")?;
                    value["changed"] = Value::Bool(result.changed);
                    value["failed"] = Value::Bool(result.status == "failed");
                    if !result.diff.is_empty() {
                        value["diff"] = Value::String(result.diff);
                    }
                    return Ok(value);
                }
                Some(v1::event::Msg::Crash(_)) => bail!("agent crashed"),
                Some(v1::event::Msg::PauseRequest(_)) => bail!("unexpected agent pause"),
                _ => {}
            }
        }
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        let event = self.lookups.redact(event)?;
        self.output.event(&event)
    }
}

mod connect;
mod scheduler;
use scheduler::run_plays;

/// Run an evaluated, validated configuration through the controller and agent protocol.
///
/// # Errors
/// Returns a redacted evaluation, connection, protocol or output error.
pub async fn run(
    config: &Value,
    options: &Options,
    runtime: &RuntimeOptions,
) -> Result<BTreeMap<String, Recap>> {
    anyhow::ensure!(
        (1..=128).contains(&runtime.jobs),
        "jobs must be between 1 and 128"
    );
    let validated = crate::model::validate_config(config)?;
    let config = &validated;
    let execution = Execution {
        inventory: config
            .get("inventory")
            .and_then(|v| v.get("hosts"))
            .and_then(Value::as_object)
            .context("missing inventory hosts")?
            .clone(),
        runtime: runtime.clone(),
        output: Arc::new(Output::new(runtime.output, runtime.log_file.as_deref())?),
        mode: options.mode,
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        lookups: Arc::new(Lookups {
            resolver: if runtime.synthetic_secrets {
                Resolver::synthetic()
            } else {
                Resolver::new(Arc::new(ProcessBackend::default()))
            },
            values: Mutex::new(Vec::new()),
        }),
    };
    let result = async {
        let (facts, failed) = connect::preconnect(config, options, &execution).await?;
        run_plays(config, &facts, options, &execution, failed).await
    }
    .await;
    let mut shutdown_errors = Vec::new();
    let connections = std::mem::take(&mut *execution.connections.lock().await);
    for (host, connection) in connections {
        match Arc::try_unwrap(connection) {
            Err(_) => {
                shutdown_errors.push((host, anyhow::anyhow!("agent connection still in use")));
            }
            Ok(connection) => {
                if let Err(error) = connection.into_inner().shutdown().await {
                    shutdown_errors.push((host, error));
                }
            }
        }
    }
    let redact_error = |error: anyhow::Error| -> anyhow::Error {
        match execution.lookups.redact(&Value::String(error.to_string())) {
            Ok(Value::String(message)) => anyhow::anyhow!(message),
            _ => anyhow::anyhow!("execution failed"),
        }
    };
    match result {
        Err(error) => {
            let mut errors = vec![error.to_string()];
            errors.extend(
                shutdown_errors
                    .into_iter()
                    .map(|(host, error)| format!("{host}: {error}")),
            );
            Err(redact_error(anyhow::anyhow!(errors.join("; "))))
        }
        Ok(mut recap) => {
            for (host, error) in shutdown_errors {
                connect::report_failure(&execution, &host, "unreachable", &error)?;
                let total = recap.entry(host).or_default();
                if total.failed == 0 {
                    total.failed = 1;
                }
            }
            for (host, total) in &recap {
                let event = json!({"host":host,"recap":{"planned":total.planned,"ok":total.ok,"changed":total.changed,"failed":total.failed,"skipped":total.skipped,"rescued":total.rescued,"ignored":total.ignored}});
                execution.output.event(&execution.lookups.redact(&event)?)?;
            }
            Ok(recap)
        }
    }
}

#[cfg(test)]
mod tests;

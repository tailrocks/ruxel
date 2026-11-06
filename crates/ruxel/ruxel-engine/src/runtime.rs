//! Connect the typed task controller to actual agent processes.
use crate::{
    controller::{self, Backend, Mode, Options, Recap, Request},
    expression::{
        self, Expr, Scope,
        provenance::{Provenance, TrackedValue},
    },
    secrets::{Lookup, ProcessBackend, Resolver},
    transport::{self, AgentConnection, LocalConnection},
};
use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Transport selection is explicit; local mode never calls SSH.
#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub agent: PathBuf,
    pub jobs: usize,
    pub no_cache: bool,
    pub connection: transport::ConnectOptions,
    pub output: OutputFormat,
    pub log_file: Option<PathBuf>,
    pub source_dir: PathBuf,
    pub synthetic_secrets: bool,
    pub local_state: Option<PathBuf>,
    pub local_agent: Option<PathBuf>,
    pub controller_state: Option<PathBuf>,
}

enum Connection {
    Local(Box<LocalConnection>),
    Remote(Box<AgentConnection>),
}
impl Connection {
    async fn send(&mut self, message: &v1::Envelope) -> Result<()> {
        match self {
            Self::Local(c) => c.send(message).await,
            Self::Remote(c) => c.send(message).await,
        }
    }
    async fn next(&mut self) -> Result<Option<v1::Event>> {
        match self {
            Self::Local(c) => c.next_event().await,
            Self::Remote(c) => c.next_event().await,
        }
    }
    async fn shutdown(self) -> Result<()> {
        match self {
            Self::Local(c) => c.shutdown().await,
            Self::Remote(c) => c.shutdown().await,
        }
    }
}

mod output;
use output::Output;
pub use output::OutputFormat;

mod lookups;
use lookups::Lookups;

#[derive(Clone)]
struct Execution {
    inventory: Map<String, Value>,
    runtime: RuntimeOptions,
    mode: Mode,
    connections: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Connection>>>>>,
    lookups: Arc<Lookups>,
    output: Arc<Output>,
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
            let (c, f) = LocalConnection::connect(
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

    fn prefetch_lookups(&self, value: &Value) -> Result<()> {
        // Prefetch independent direct lookup nodes only. Calls retain lazy branch semantics.
        fn direct_lookups(value: &Value, found: &mut Vec<Lookup>) -> Result<()> {
            if let Some(kind) = value.get("kind").and_then(Value::as_str) {
                if kind == "lookup" {
                    found.push(Lookups::lookup(&serde_json::from_value::<Expr>(
                        value.clone(),
                    )?)?);
                    return Ok(());
                }
                if ["literal", "ref", "call", "list", "map"].contains(&kind) {
                    return Ok(());
                }
            }
            match value {
                Value::Array(items) => {
                    for item in items {
                        direct_lookups(item, found)?;
                    }
                }
                Value::Object(items) => {
                    for item in items.values() {
                        direct_lookups(item, found)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut lookups = Vec::new();
        direct_lookups(value, &mut lookups)?;
        self.lookups
            .resolver
            .prefetch(&lookups)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    fn evaluate_value_tracked(&self, value: &Value, scope: &TrackedValue) -> Result<TrackedValue> {
        if let Some(kind) = value.get("kind").and_then(Value::as_str)
            && ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind)
        {
            return expression::provenance::evaluate_tracked(
                &serde_json::from_value::<Expr>(value.clone())?,
                scope,
                self.lookups.as_ref(),
            );
        }
        match value {
            Value::Array(items) => {
                let items = items
                    .iter()
                    .map(|v| self.evaluate_value_tracked(v, scope))
                    .collect::<Result<Vec<_>>>()?;
                Ok(TrackedValue {
                    value: Value::Array(items.iter().map(|v| v.value.clone()).collect()),
                    provenance: Provenance::Array(
                        items.into_iter().map(|v| v.provenance).collect(),
                    ),
                })
            }
            Value::Object(items) => Ok(TrackedValue::object(
                items
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.evaluate_value_tracked(v, scope)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
            )),
            _ => Ok(TrackedValue::public(value.clone())),
        }
    }

    fn evaluate_value(&self, value: &Value, scope: &Scope) -> Result<Value> {
        if let Some(kind) = value.get("kind").and_then(Value::as_str)
            && ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind)
        {
            return expression::evaluate(
                &serde_json::from_value::<Expr>(value.clone())?,
                scope,
                self.lookups.as_ref(),
            );
        }
        match value {
            Value::Array(items) => items
                .iter()
                .map(|v| self.evaluate_value(v, scope))
                .collect(),
            Value::Object(items) => items
                .iter()
                .map(|(k, v)| Ok((k.clone(), self.evaluate_value(v, scope)?)))
                .collect::<Result<Map<_, _>>>()
                .map(Value::Object),
            _ => Ok(value.clone()),
        }
    }
}
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
        lookups: Arc::new(Lookups {
            resolver: if runtime.synthetic_secrets {
                Resolver::synthetic()
            } else {
                Resolver::new(Arc::new(ProcessBackend::default()))
            },
            values: Mutex::new(Vec::new()),
        }),
    };
    let mut facts = BTreeMap::new();
    let result = async {
        if options.mode != Mode::Plan {

            for host in controller::selected_hosts(config, options)? {
                let gathered = execution.connect(&host).await?;
                facts.insert(host.clone(),json!({"default_ipv4_interface":gathered.default_ipv4_interface,"distribution_release":gathered.distribution_release,"architecture":gathered.architecture,"hostname":gathered.hostname}));

            }
        }
        run_plays(config, &facts, options, &execution).await
    }.await;
    let mut shutdown_errors = Vec::new();
    let connections = std::mem::take(&mut *execution.connections.lock().await);
    for (host, connection) in connections {
        match Arc::try_unwrap(connection) {
            Err(_) => shutdown_errors.push(format!("{host}: agent connection still in use")),
            Ok(connection) => {
                if let Err(error) = connection.into_inner().shutdown().await {
                    shutdown_errors.push(format!("{host}: {error}"));
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
            shutdown_errors.insert(0, error.to_string());
            Err(redact_error(anyhow::anyhow!(shutdown_errors.join("; "))))
        }
        Ok(recap) => {
            if !shutdown_errors.is_empty() {
                return Err(redact_error(anyhow::anyhow!(shutdown_errors.join("; "))));
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

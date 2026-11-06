//! Connect the typed task controller to actual agent processes.
use crate::{
    controller::{self, Backend, Mode, Options, Recap, Request},
    expression::{self, Expr, LookupResolver, Scope},
    secrets::{Lookup, ProcessBackend, Resolver},
    transport::{self, AgentConnection, LocalConnection},
};
use anyhow::{Context, Result, bail};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Transport selection is explicit; local mode never calls SSH.
#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub agent: PathBuf,
    pub source_dir: PathBuf,
    pub synthetic_secrets: bool,
    pub local_state: Option<PathBuf>,
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

struct Lookups {
    resolver: Resolver,
    values: Mutex<Vec<String>>,
}
impl Lookups {
    fn lookup(expr: &Expr) -> Result<Lookup> {
        let Expr::Lookup {
            provider,
            key,
            vault,
            item,
            section,
            field,
            reference,
            argv,
        } = expr
        else {
            bail!("expected lookup");
        };
        let required = |value: &Option<String>| value.clone().context("missing lookup selector");
        Ok(match provider.as_str() {
            "fnox" => Lookup::Fnox {
                key: required(key)?,
            },
            "op" if reference.is_some() => Lookup::OnePasswordUri {
                reference: required(reference)?,
            },
            "op" => Lookup::OnePassword {
                vault: required(vault)?,
                item: required(item)?,
                field: required(field)?,
                section: section.clone(),
            },
            "command" => Lookup::Command { argv: argv.clone() },
            "pipe" => Lookup::Pipe {
                command: required(key)?,
            },
            "synthetic" => Lookup::Synthetic {
                value: required(key)?,
            },
            _ => bail!("unknown lookup provider"),
        })
    }
    fn redact(&self, value: &Value) -> Result<Value> {
        fn scrub(value: &Value, secrets: &[String]) -> Value {
            match value {
                Value::String(text) => Value::String(
                    secrets
                        .iter()
                        .filter(|s| !s.is_empty())
                        .fold(text.clone(), |text, secret| {
                            text.replace(secret, "[REDACTED]")
                        }),
                ),
                Value::Array(items) => {
                    Value::Array(items.iter().map(|v| scrub(v, secrets)).collect())
                }
                Value::Object(items) => Value::Object(
                    items
                        .iter()
                        .map(|(key, value)| {
                            let key = secrets
                                .iter()
                                .filter(|s| !s.is_empty())
                                .fold(key.clone(), |key, secret| key.replace(secret, "[REDACTED]"));
                            (key, scrub(value, secrets))
                        })
                        .collect(),
                ),
                _ => value.clone(),
            }
        }
        let secrets = self
            .values
            .lock()
            .map_err(|_| anyhow::anyhow!("redactor unavailable"))?;
        Ok(scrub(value, &secrets))
    }
}
impl LookupResolver for Lookups {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let value = self
            .resolver
            .resolve(&Self::lookup(expr)?)
            .map_err(anyhow::Error::msg)?;
        self.values
            .lock()
            .map_err(|_| anyhow::anyhow!("redactor unavailable"))?
            .push(value.clone());
        Ok(Value::String(value))
    }
}

struct Execution {
    inventory: Map<String, Value>,
    runtime: RuntimeOptions,
    mode: Mode,
    connections: BTreeMap<String, Connection>,
    lookups: Lookups,
}
impl Execution {
    async fn connect(&mut self, host: &str) -> Result<v1::Facts> {
        let spec = self
            .inventory
            .get(host)
            .context("selected host is missing")?;
        let check = matches!(self.mode, Mode::Check | Mode::Diff);
        let diff = self.mode == Mode::Diff;
        let (connection, gathered) = if let Some(state) = &self.runtime.local_state {
            // Host names are inventory keys, never accepted as filesystem paths.
            let path = state.join(blake3::hash(host.as_bytes()).to_hex().as_str());
            let (c, f) =
                LocalConnection::connect(&self.runtime.agent, &path, "pkl-local", check, diff)
                    .await?;
            (Connection::Local(Box::new(c)), f)
        } else {
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
                port,
                ..Default::default()
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
        self.connections.insert(host.to_owned(), connection);
        Ok(gathered)
    }

    fn evaluate_value(&self, value: &Value, scope: &Scope) -> Result<Value> {
        if let Some(kind) = value.get("kind").and_then(Value::as_str)
            && ["literal", "ref", "call", "list", "map", "lookup"].contains(&kind)
        {
            return expression::evaluate(
                &serde_json::from_value::<Expr>(value.clone())?,
                scope,
                &self.lookups,
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
impl Backend for Execution {
    fn evaluate(&mut self, value: &Value, map: &Map<String, Value>) -> Result<Value> {
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
        let scope = Scope {
            facts: map.get("facts").cloned().unwrap_or(Value::Null),
            results: map.get("results").cloned().unwrap_or(Value::Null),
            variables: map.get("vars").cloned().unwrap_or(Value::Null),
            item: map.get("item").cloned().unwrap_or(Value::Null),
            extra: map.clone(),
        };
        self.evaluate_value(value, &scope)
    }
    async fn execute(&mut self, mut request: Request) -> Result<Value> {
        if matches!(request.task.module.as_str(), "copy" | "template") {
            for iteration in &mut request.task.iterations {
                let mut params: Map<String, Value> =
                    serde_json::from_slice(&iteration.params_json)?;
                if let Some(src) = params.remove("src").filter(|v| !v.is_null()) {
                    let source = self
                        .runtime
                        .source_dir
                        .join(src.as_str().context("source path must be text")?);
                    let content =
                        std::fs::read_to_string(source).context("read controller source file")?;
                    params.insert("content".into(), Value::String(content));
                    iteration.params_json = serde_json::to_vec(&params)?;
                }
            }
        }
        if !self.connections.contains_key(&request.host) {
            self.connect(&request.host).await?;
        }
        let connection = self
            .connections
            .get_mut(&request.host)
            .context("execution host is not connected")?;
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
        writeln!(std::io::stdout().lock(), "{event}").context("write execution event")
    }
}

/// Run an evaluated, validated configuration through the controller and agent protocol.
///
/// # Errors
/// Returns a redacted evaluation, connection, protocol or output error.
pub async fn run(
    config: &Value,
    options: &Options,
    runtime: &RuntimeOptions,
) -> Result<BTreeMap<String, Recap>> {
    let validated = crate::model::validate_config(config)?;
    let config = &validated;
    let mut execution = Execution {
        inventory: config
            .get("inventory")
            .and_then(|v| v.get("hosts"))
            .and_then(Value::as_object)
            .context("missing inventory hosts")?
            .clone(),
        runtime: runtime.clone(),
        mode: options.mode,
        connections: BTreeMap::new(),
        lookups: Lookups {
            resolver: if runtime.synthetic_secrets {
                Resolver::synthetic()
            } else {
                Resolver::new(Arc::new(ProcessBackend))
            },
            values: Mutex::new(Vec::new()),
        },
    };
    let mut facts = BTreeMap::new();
    let result = async {
        if options.mode != Mode::Plan {

            for host in controller::selected_hosts(config, options)? {
                let gathered = execution.connect(&host).await?;
                facts.insert(host.clone(),json!({"default_ipv4_interface":gathered.default_ipv4_interface,"distribution_release":gathered.distribution_release,"architecture":gathered.architecture,"hostname":gathered.hostname}));

            }
        }
        controller::run(config,&facts,options,&mut execution).await
    }.await;
    let mut shutdown_error = None;
    for (_, connection) in std::mem::take(&mut execution.connections) {
        if let Err(error) = connection.shutdown().await {
            shutdown_error = Some(error);
        }
    }
    let redact_error = |error: anyhow::Error| -> anyhow::Error {
        match execution.lookups.redact(&Value::String(error.to_string())) {
            Ok(Value::String(message)) => anyhow::anyhow!(message),
            _ => anyhow::anyhow!("execution failed"),
        }
    };
    match result {
        Err(error) => Err(redact_error(error)),
        Ok(recap) => {
            if let Some(error) = shutdown_error {
                return Err(redact_error(error));
            }
            Ok(recap)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_preserves_json_structure_for_short_and_escaped_secrets() -> Result<()> {
        let lookups = Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(vec!["true".into(), "quoted\"secret".into()]),
        };
        let result = lookups
            .redact(&json!({"value":true,"nested":["true","quoted\"secret"],"true-key":1}))?;
        assert_eq!(result["value"], true);
        assert_eq!(result["nested"], json!(["[REDACTED]", "[REDACTED]"]));
        assert_eq!(result["[REDACTED]-key"], 1);
        Ok(())
    }
}

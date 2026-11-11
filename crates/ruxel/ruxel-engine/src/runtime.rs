use crate::{
    controller::{Backend, Mode, Options, Recap, Request},
    expression::{Scope, provenance::TrackedValue},
    secrets::{self, ProcessBackend, Resolver},
    transport,
};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc};

mod options;
pub use options::RuntimeOptions;

mod connection;
use connection::Connection;

mod execution;

mod output;
use output::Output;
pub use output::OutputFormat;

mod lookups;
use lookups::Lookups;
pub(crate) mod lazy_vars;
mod resources;
use resources::Resources;
mod copy_sources;
mod delegates;

#[derive(Clone)]
struct Execution {
    connection_failures: u64,
    conservative_sensitive: Arc<std::sync::atomic::AtomicBool>,
    effect_gate: Arc<tokio::sync::RwLock<()>>,
    inventory: Map<String, Value>,
    runtime: RuntimeOptions,
    mode: Mode,
    connections: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Connection>>>>>,
    lookups: Arc<Lookups>,
    output: Arc<Output>,
    resources: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Resources>>>>>,
    source_gate: Arc<tokio::sync::RwLock<()>>,
    template_root: Arc<templates::SourceRoot>,
    copy_root: Arc<copy_sources::SourceRoot>,
}
#[cfg(test)]
#[path = "runtime/tests/effects.rs"]
mod effect_tests;
mod evaluation;
mod templates;

mod connect;
mod scheduler;
use scheduler::run_plays;

/// # Errors
/// Returns a redacted evaluation, connection, protocol or output error.
#[expect(
    clippy::too_many_lines,
    reason = "run owns startup, execution, and connection cleanup"
)]
pub async fn run(
    config: &Value,
    options: &Options,
    runtime: &RuntimeOptions,
) -> Result<BTreeMap<String, Recap>> {
    anyhow::ensure!(
        (1..=128).contains(&runtime.jobs),
        "jobs must be between 1 and 128"
    );
    let validated = crate::model::validate_config(config).map_err(|error| {
        if options.mode == Mode::Plan {
            anyhow::anyhow!("plan validation failed")
        } else {
            error
        }
    })?;
    let config = &validated;
    let copy_root = Arc::new(copy_sources::SourceRoot::open(&runtime.source_dir)?);
    let template_root = Arc::new(copy_root.template_root(&runtime.source_dir)?);
    let execution = Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(
            options.mode == Mode::Plan,
        )),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
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
        template_root,
        copy_root,
        // Synthetic mode is wired to plan, check and diff — it is the
        // mode's property, not an opt-in flag. Every secret answers a
        // selector-derived placeholder, no store is contacted, and nothing
        // is generated: synthetic mode is rotation-blind by design (a
        // placeholder derives from the selector, never the value). Only
        // apply resolves real values, and only apply generates.
        lookups: Arc::new(Lookups::new(
            if options.mode != Mode::Apply || runtime.synthetic_secrets {
                Resolver::synthetic()
            } else {
                Resolver::new(Arc::new(ProcessBackend::default()))
            },
        )),
    };
    let result = async {
        // Generation happens once per run, on the controller, before any
        // secret is resolved and before the first connection. Only apply
        // generates; plan, check and diff run synthetic and never reach
        // here. A missing generated key becomes a selector-derived
        // placeholder elsewhere.
        if options.mode == Mode::Apply && !runtime.synthetic_secrets {
            let requests = secrets::generated_requests(config);
            let output = execution.output.clone();
            let lookups = execution.lookups.clone();
            execution
                .lookups
                .resolver
                .ensure_generated(&requests, &mut |key, format| {
                    let event = json!({"event":"secret_generated","name":key,"format":format,"message":format!("generated secret {key} (stored in fnox)")});
                    if let Err(error) = output.event(&lookups.redact(&event)) {
                        // Names the key and the format, never a value.
                        eprintln!("generated secret event could not be written: {error}");
                    }
                })
                .map_err(anyhow::Error::msg)?;
        }
        let (facts, failed) = connect::preconnect(config, options, &execution)
            .await
            .inspect_err(|_| {
                execution
                    .conservative_sensitive
                    .store(true, std::sync::atomic::Ordering::Release);
            })?;
        let checked = execution.connections.lock().await.keys().cloned().collect();
        if options.mode == Mode::Plan
            || delegates::requires_conservative(config, &checked, !failed.is_empty())
        {
            execution
                .conservative_sensitive
                .store(true, std::sync::atomic::Ordering::Release);
        }
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
        if execution.conservative_sensitive() {
            return anyhow::anyhow!("sensitive execution failed");
        }
        match execution.lookups.redact(&Value::String(error.to_string())) {
            Value::String(message) => anyhow::anyhow!(message),
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
                total.unreachable = total.unreachable.max(1);
                if total.failed == 0 {
                    total.failed = 1;
                }
            }
            for (host, total) in &recap {
                let visible_host = if execution.conservative_sensitive() {
                    "[censored]"
                } else {
                    host
                };
                let mut event = json!({"host":visible_host,"recap":{"planned":total.planned,"ok":total.ok,"changed":total.changed,"failed":total.failed,"skipped":total.skipped,"rescued":total.rescued,"ignored":total.ignored}});
                if total.unreachable != 0 {
                    event["recap"]["unreachable"] = json!(total.unreachable);
                }
                execution.output.event(&execution.lookups.redact(&event))?;
            }
            Ok(recap)
        }
    }
}

#[cfg(test)]
mod tests;

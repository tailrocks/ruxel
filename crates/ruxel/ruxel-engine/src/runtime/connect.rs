use super::Execution;
use crate::controller::{self, Backend, Mode, Options, Recap};
use anyhow::Result;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::future::Future;

#[cfg(test)]
#[path = "tests/preconnect.rs"]
mod tests;

fn bounded<F, Fut, T>(
    hosts: Vec<String>,
    jobs: usize,
    connect: F,
) -> impl futures_util::Stream<Item = (String, T)>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = T>,
{
    stream::iter(hosts)
        .map(move |host| {
            let future = connect(host.clone());
            async move { (host, future.await) }
        })
        .buffered(jobs.max(1))
}

pub(super) fn report_failure(
    execution: &Execution,
    host: &str,
    event: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let (host, message) = if execution.conservative_sensitive() {
        ("[censored]", "sensitive execution failed".to_owned())
    } else {
        (host, error.to_string())
    };
    let value = if event == "unreachable" {
        json!({"host":host,"event":event,"unreachable":true,"changed":false,"msg":message})
    } else {
        json!({"host":host,"event":event,"failed":true,"msg":message})
    };
    execution.output.event(&execution.lookups.redact(&value))
}

pub(super) async fn preconnect(
    config: &Value,
    options: &Options,
    execution: &Execution,
) -> Result<(BTreeMap<String, Value>, BTreeMap<String, Recap>)> {
    let mut facts = BTreeMap::new();
    let mut failed = BTreeMap::new();
    let hosts = controller::selected_hosts(config, options)?;
    if options.mode == Mode::Plan {
        return Ok((facts, failed));
    }
    let gate = execution.source_gate.write().await;
    let results = bounded(hosts, execution.runtime.jobs, |host| {
        let gate = &gate;
        async move { execution.connect_guarded(&host, gate).await }
    });
    futures_util::pin_mut!(results);
    while let Some((host, result)) = results.next().await {
        match result {
            Ok(gathered) => {
                facts.insert(host, json!({"default_ipv4_interface":gathered.default_ipv4_interface,"distribution_release":gathered.distribution_release,"architecture":gathered.architecture,"hostname":gathered.hostname}));
            }
            Err(error) => {
                execution
                    .conservative_sensitive
                    .store(true, std::sync::atomic::Ordering::Release);
                report_failure(execution, &host, "unreachable", &error)?;
                failed.insert(
                    host,
                    Recap {
                        failed: 1,
                        unreachable: 1,
                        ..Recap::default()
                    },
                );
            }
        }
    }
    Ok((facts, failed))
}

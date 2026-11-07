use super::Execution;
use crate::controller::{self, Mode, Options, Recap};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn report_failure(
    execution: &Execution,
    host: &str,
    event: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let (host, message) = if execution.mode == Mode::Plan {
        ("[censored]", "plan failed".to_owned())
    } else {
        (host, error.to_string())
    };
    let value = if event == "unreachable" {
        json!({"host":host,"event":event,"unreachable":true,"changed":false,"msg":message})
    } else {
        json!({"host":host,"event":event,"failed":true,"msg":message})
    };
    execution.output.event(&execution.lookups.redact(&value)?)
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
    for host in hosts {
        match execution.connect(&host).await {
            Ok(gathered) => {
                facts.insert(host, json!({"default_ipv4_interface":gathered.default_ipv4_interface,"distribution_release":gathered.distribution_release,"architecture":gathered.architecture,"hostname":gathered.hostname}));
            }
            Err(error) => {
                report_failure(execution, &host, "unreachable", &error)?;
                failed.insert(
                    host,
                    Recap {
                        failed: 1,
                        ..Recap::default()
                    },
                );
            }
        }
    }
    Ok((facts, failed))
}

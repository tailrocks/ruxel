use super::Execution;
use crate::controller::{self, Options, Recap};
use anyhow::{Context, Result};
use futures_util::{StreamExt, stream};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) async fn run_plays(
    config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    execution: &Execution,
    mut totals: BTreeMap<String, Recap>,
) -> Result<BTreeMap<String, Recap>> {
    let mut set_facts = BTreeMap::new();
    for play in config
        .get("plays")
        .and_then(Value::as_array)
        .context("missing plays")?
    {
        let mut play_config = config.clone();
        play_config["plays"] = Value::Array(vec![play.clone()]);
        let hosts: Vec<_> = controller::selected_hosts(&play_config, options)?
            .into_iter()
            .filter(|host| totals.get(host).is_none_or(|recap| recap.failed == 0))
            .collect();
        let runs = hosts.into_iter().map(|host| {
            let mut host_facts = BTreeMap::new();
            if let Some(facts) = set_facts.remove(&host) {
                host_facts.insert(host.clone(), facts);
            }
            let mut backend = execution.clone();
            let mut options = options.clone();
            options.hosts = vec![host.clone()];
            let config = &play_config;
            async move {
                let result =
                    controller::run(config, facts, &options, &mut backend, &mut host_facts).await;
                (host, result, host_facts)
            }
        });
        let completed: Vec<_> = stream::iter(runs)
            .buffer_unordered(execution.runtime.jobs)
            .collect()
            .await;
        for (host, result, host_facts) in completed {
            set_facts.extend(host_facts);
            match result {
                Err(failure) => {
                    if let Some(partial) = failure.downcast_ref::<controller::RunFailure>() {
                        for (name, recap) in &partial.recaps {
                            let total = totals.entry(name.clone()).or_default();
                            total.ok += recap.ok;
                            total.planned += recap.planned;
                            total.changed += recap.changed;
                            total.failed += recap.failed;
                            total.skipped += recap.skipped;
                            total.rescued += recap.rescued;
                            total.ignored += recap.ignored;
                        }
                    }
                    super::connect::report_failure(execution, &host, "host_failed", &failure)?;
                    totals.entry(host).or_default().failed += 1;
                }
                Ok(recaps) => {
                    for (host, recap) in recaps {
                        let total = totals.entry(host).or_default();
                        total.ok += recap.ok;
                        total.planned += recap.planned;
                        total.changed += recap.changed;
                        total.failed += recap.failed;
                        total.skipped += recap.skipped;
                        total.rescued += recap.rescued;
                        total.ignored += recap.ignored;
                    }
                }
            }
        }
    }
    Ok(totals)
}

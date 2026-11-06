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
) -> Result<BTreeMap<String, Recap>> {
    let mut totals: BTreeMap<String, Recap> = BTreeMap::new();
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
            let mut backend = execution.clone();
            let mut options = options.clone();
            options.hosts = vec![host.clone()];
            let config = &play_config;
            async move {
                controller::run(config, facts, &options, &mut backend)
                    .await
                    .map_err(|error| anyhow::anyhow!("{host}: {error}"))
            }
        });
        // Collect every result before returning an error so sibling work cannot be cancelled halfway.
        let completed: Vec<_> = stream::iter(runs)
            .buffer_unordered(execution.runtime.jobs)
            .collect()
            .await;
        let mut errors = Vec::new();
        for result in completed {
            match result {
                Err(failure) => {
                    errors.push(failure.to_string());
                }
                Ok(recaps) => {
                    for (host, recap) in recaps {
                        let total = totals.entry(host).or_default();
                        total.ok += recap.ok;
                        total.changed += recap.changed;
                        total.failed += recap.failed;
                        total.skipped += recap.skipped;
                        total.rescued += recap.rescued;
                        total.ignored += recap.ignored;
                    }
                }
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(errors.join("; "));
        }
    }
    Ok(totals)
}

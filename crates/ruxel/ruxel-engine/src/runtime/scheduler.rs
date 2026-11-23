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
        // Frames carry the full configuration's host-applicable static
        // set, not the current play's: later plays hide from per-play scans.
        let full_config = std::sync::Arc::new((*config).clone());
        let runs = hosts.into_iter().map(|host| {
            let full_config = full_config.clone();
            let mut host_facts = BTreeMap::new();
            if let Some(facts) = set_facts.remove(&host) {
                host_facts.insert(host.clone(), facts);
            }
            let mut backend = execution.clone();
            let mut options = options.clone();
            options.hosts = vec![host.clone()];
            let config = &play_config;
            async move {
                let result = controller::run_with_full_config(
                    config,
                    &full_config,
                    facts,
                    &options,
                    &mut backend,
                    &mut host_facts,
                )
                .await;
                (host, result, host_facts)
            }
        });
        let completed: Vec<_> = stream::iter(runs)
            .buffer_unordered(execution.runtime.jobs)
            .collect()
            .await;
        let mut play_hosts = Vec::new();
        for (host, result, host_facts) in completed {
            set_facts.extend(host_facts);
            play_hosts.push(host.clone());
            match result {
                Err(failure) => {
                    if let Some(partial) = failure.downcast_ref::<controller::RunFailure>() {
                        for (name, recap) in &partial.recaps {
                            merge_recap(totals.entry(name.clone()).or_default(), recap);
                        }
                    }
                    super::connect::report_failure(execution, &host, "host_failed", &failure)?;
                    let total = totals.entry(host).or_default();
                    total.failed += 1;
                    // The host's session stopped: its later plays never
                    // run, so the shutdown must not prune the tasks it
                    // never sent.
                    total.aborted = true;
                }
                Ok(recaps) => {
                    for (host, recap) in recaps {
                        merge_recap(totals.entry(host).or_default(), &recap);
                    }
                }
            }
        }
        // An incomplete origin marks its destinations incomplete too:
        // a delegated host with a clean direct recap of its own must
        // still shut down filtered, or its unfiltered Done prunes the
        // delegated records the origin never resent.
        for host in &play_hosts {
            if totals.get(host).is_some_and(|total| total.filtered || total.aborted) {
                controller::mark_delegation_destinations(&mut totals, play, host);
            }
        }
    }
    Ok(totals)
}

/// One recap folded into a host's total: the partial recaps a stopped
/// session reports and the recaps a completed play reports merge the
/// same way.
fn merge_recap(total: &mut Recap, recap: &Recap) {
    total.ok += recap.ok;
    total.planned += recap.planned;
    total.changed += recap.changed;
    total.failed += recap.failed;
    total.unreachable += recap.unreachable;
    total.skipped += recap.skipped;
    total.rescued += recap.rescued;
    total.ignored += recap.ignored;
    total.filtered |= recap.filtered;
    total.aborted |= recap.aborted;
    total.delegated.extend(recap.delegated.iter().cloned());
}

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
            if totals
                .get(host)
                .is_some_and(|total| total.filtered || total.aborted)
            {
                controller::mark_delegation_destinations(&mut totals, play, host);
            }
        }
    }
    // A destination whose direct recap lands only in a later play is
    // absent when its origin's play ends, so the per-play propagation
    // above cannot mark it — and the later play never revisits the
    // origin's delegation. Revisit every play now that all recaps exist.
    revisit_earlier_delegations(&mut totals, config);
    Ok(totals)
}

/// Every incomplete origin marks its destinations again once all plays
/// ran: a destination whose direct recap lands only in a later play is
/// absent from `totals` when its origin's play ends, so the per-play
/// propagation cannot mark it — and the later play never revisits the
/// origin's delegation. The revisit runs over the plays of the full
/// configuration, the way the single-play controller run does — but
/// only the plays the origin was actually a target of: an unrelated
/// incomplete origin must not mark another play's destinations (see
/// [`controller::origin_targeted`]).
fn revisit_earlier_delegations(totals: &mut BTreeMap<String, Recap>, config: &Value) {
    let incomplete: Vec<String> = totals
        .iter()
        .filter(|(_, total)| total.filtered || total.aborted)
        .map(|(host, _)| host.clone())
        .collect();
    for play in config
        .get("plays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for origin in &incomplete {
            if controller::origin_targeted(play, config, origin) {
                controller::mark_delegation_destinations(totals, play, origin);
            }
        }
    }
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

#[cfg(test)]
mod scheduler_revisit_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn earlier_delegation_revisited_after_later_play() {
        // Thread 341: play 1 filters origin `a`, whose literal
        // delegation names `b`; `b` runs its direct recap only in play
        // 2. The per-play propagation at the end of play 1 cannot mark
        // `b` — it is absent from `totals` — and play 2 never revisits
        // the origin's delegation, so the old sequence leaves `b`
        // clean to send an unfiltered Done.
        let play1 = json!({"targets": ["a"], "tasks": [
            {"name": "delegated", "delegateTo": "b", "delegateAllow": ["b"],
             "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let play2 = json!({"targets": ["b"], "tasks": [
            {"name": "direct", "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let config = json!({"plays": [play1.clone(), play2.clone()]});

        let mut totals = BTreeMap::new();
        totals.insert(
            "a".to_string(),
            Recap {
                filtered: true,
                ..Recap::default()
            },
        );
        // The old per-play propagation at the end of play 1: `b` is
        // absent, so nothing is marked.
        controller::mark_delegation_destinations(&mut totals, &play1, "a");
        assert!(
            !totals.contains_key("b"),
            "the old per-play mark cannot reach a destination with no recap yet"
        );
        // Play 2 reports `b` clean, and its own play carries no
        // delegation, so the old per-play mark at the end of play 2 is
        // still a no-op.
        totals.insert("b".to_string(), Recap::default());
        controller::mark_delegation_destinations(&mut totals, &play2, "b");
        assert!(
            !totals["b"].filtered,
            "reproduces thread 341: the later play never revisits the origin's delegation"
        );
        // The scheduler's end-of-run revisit marks it.
        revisit_earlier_delegations(&mut totals, &config);
        assert!(
            totals["b"].filtered,
            "a destination delegated from a filtered origin in an earlier play stays filtered"
        );
    }

    #[test]
    fn unrelated_origins_do_not_mark_other_plays_destinations() {
        // Thread 362: the revisit scopes its play-by-origin
        // cross-product to the plays the origin was actually a target
        // of. Origin `a` is filtered; the second play targets only `c`
        // and delegates to `d`. The unscoped revisit marked `d` through
        // the (second play, `a`) pair although `a` never ran there.
        let play1 = json!({"targets": ["a"], "tasks": [
            {"name": "delegated", "delegateTo": "b", "delegateAllow": ["b"],
             "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let play2 = json!({"targets": ["c"], "tasks": [
            {"name": "other", "delegateTo": "d", "delegateAllow": ["d"],
             "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let config = json!({"plays": [play1.clone(), play2.clone()]});

        let mut totals = BTreeMap::new();
        totals.insert(
            "a".to_string(),
            Recap {
                filtered: true,
                ..Recap::default()
            },
        );
        totals.insert("b".to_string(), Recap::default());
        totals.insert("c".to_string(), Recap::default());
        totals.insert("d".to_string(), Recap::default());
        revisit_earlier_delegations(&mut totals, &config);
        assert!(
            totals["b"].filtered,
            "the origin's own play still marks its destination"
        );
        assert!(
            !totals["d"].filtered,
            "an unrelated play's destination stays clean"
        );
        assert!(
            !totals["c"].filtered,
            "an unrelated origin recap stays clean"
        );
    }

    #[test]
    fn group_targets_count_as_actually_targeted() {
        // The scoping expands selectors the way the run selects hosts,
        // so an origin targeted through a group still marks its play's
        // destinations — while a play whose group excludes the origin
        // does not lend the origin its destinations.
        let play1 = json!({"targets": ["web"], "tasks": [
            {"name": "delegated", "delegateTo": "b", "delegateAllow": ["b"],
             "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let play2 = json!({"targets": ["db"], "tasks": [
            {"name": "other", "delegateTo": "d", "delegateAllow": ["d"],
             "operation": {"kind": "command", "argv": ["true"]}}
        ]});
        let config = json!({
            "inventory": {
                "hosts": {"a": {}, "b": {}, "c": {}, "d": {}},
                "groups": {"web": {"hosts": ["a"]}, "db": {"hosts": ["c"]}}
            },
            "plays": [play1.clone(), play2.clone()]
        });

        let mut totals = BTreeMap::new();
        totals.insert(
            "a".to_string(),
            Recap {
                filtered: true,
                ..Recap::default()
            },
        );
        totals.insert("b".to_string(), Recap::default());
        totals.insert("c".to_string(), Recap::default());
        totals.insert("d".to_string(), Recap::default());
        revisit_earlier_delegations(&mut totals, &config);
        assert!(
            totals["b"].filtered,
            "an origin targeted through a group still marks its play's destination"
        );
        assert!(
            !totals["d"].filtered,
            "a play whose group excludes the origin does not mark through it"
        );
    }
}

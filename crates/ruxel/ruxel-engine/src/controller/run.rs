//! One play's controller run, over the configuration the scheduler split
//! it out of: the run carries the full configuration's host-applicable
//! static sysctl declarations into every frame.

use super::selection::{expand_selectors, group_hosts, ordered_groups, selected_hosts};
use super::{
    Backend, ContextLayer, Mode, Options, Recap, RunFailure, Runner, TrackedValue, array, bail,
    object, string,
};
use super::{plan, static_sysctl, validation};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// # Errors
/// Returns an error if inventory, expression evaluation, or execution fails.
pub async fn run<B: Backend>(
    config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    backend: &mut B,
    set_facts: &mut BTreeMap<String, TrackedValue>,
) -> Result<BTreeMap<String, Recap>> {
    run_with_full_config(config, config, facts, options, backend, set_facts).await
}

/// The controller run over one play's configuration, with the static
/// sysctl declarations of the full configuration the scheduler split
/// the play out of: the per-play scan would never see declarations in
/// later plays (workflow stages), so every frame carries the
/// host-applicable set of the whole configuration instead.
///
/// # Errors
/// Returns an error if inventory, expression evaluation, or execution fails.
#[expect(
    clippy::too_many_lines,
    reason = "host execution preserves play order and shared scope"
)]
pub async fn run_with_full_config<B: Backend>(
    config: &Value,
    full_config: &Value,
    facts: &BTreeMap<String, Value>,
    options: &Options,
    backend: &mut B,
    set_facts: &mut BTreeMap<String, TrackedValue>,
) -> Result<BTreeMap<String, Recap>> {
    validation::validate(config)?;
    selected_hosts(config, options)?;
    let inventory = config.get("inventory").context("missing inventory")?;
    let hosts = object(inventory, "hosts");
    let groups = object(inventory, "groups");
    let mut totals = BTreeMap::new();
    for play in array(config, "plays") {
        let selectors = array(play, "targets")
            .iter()
            .map(|value| value.as_str().context("target must be a string"))
            .collect::<Result<Vec<_>>>()?;
        let targets = expand_selectors(&selectors, &hosts, &groups)?;
        let filtered: BTreeSet<_> = selected_hosts(config, options)?.into_iter().collect();
        for host in targets {
            if !hosts.contains_key(&host) {
                bail!("unknown inventory host {host:?}");
            }
            if !filtered.contains(&host) {
                continue;
            }
            if options.mode == Mode::Plan {
                let inherited: Vec<String> = array(play, "tags")
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let mut planned = plan::plan_tasks(
                    backend,
                    &host,
                    array(play, "preTasks"),
                    "preTask",
                    &options.tags,
                    &inherited,
                )?;
                planned += plan::plan_tasks(
                    backend,
                    &host,
                    array(play, "tasks"),
                    "task",
                    &options.tags,
                    &inherited,
                )?;
                plan::plan_handlers(backend, &host, array(play, "handlers"))?;
                totals.entry(host).or_insert_with(Recap::default).planned += planned;
                continue;
            }
            let mut scope = Map::new();
            scope.insert("inventoryHost".into(), Value::String(host.clone()));
            scope.insert(
                "facts".into(),
                facts.get(&host).cloned().unwrap_or_else(|| json!({})),
            );
            let connection_failures_before = backend.connection_failures();
            let mut runner = Runner {
                backend,
                options,
                host: host.clone(),
                scope: TrackedValue::public(Value::Object(scope)),
                handlers: array(play, "handlers").to_vec(),
                plan_sysctl: static_sysctl::static_sysctl_declarations_for_host(
                    full_config,
                    &host,
                    &options.tags,
                ),
                notified: BTreeMap::new(),
                handling: false,
                recap: Recap::default(),
                next_id: 1,
                filtered: false,
            };
            if let Some(persisted) = set_facts.get(&host) {
                runner.scope.insert("setFacts".into(), persisted.clone())?;
            }
            let mut layer = ContextLayer {
                control_sensitive: runner.backend.conservative_sensitive(),
                ..ContextLayer::default()
            };
            for name in ordered_groups(&groups)? {
                if group_hosts(&name, &groups, &mut BTreeSet::new())?.contains(&host) {
                    layer.definitions.push(object(&groups[&name], "vars"));
                }
            }
            layer.definitions.push(object(&hosts[&host], "vars"));
            layer.definitions.push(object(play, "vars"));
            layer.tags = array(play, "tags")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            layer.environment = object(play, "environment");
            layer.become_user = string(play, "becomeUser").map(str::to_owned);
            let execution: Result<bool> = async {
                // A handler that stops the flush without an
                // execution error ends the session the same way a
                // failed task does: the pending handlers are never
                // sent, so the recap below carries the aborted mark
                // and the shutdown filters Done.
                let mut success = runner.tasks(array(play, "preTasks"), &layer).await?
                    && runner.tasks(array(play, "tasks"), &layer).await?;
                if success {
                    success = runner.run_handlers(&layer).await?;
                }
                Ok(success)
            }
            .await;
            if let Some(persisted) = runner.scope.get("setFacts") {
                set_facts.insert(host.clone(), persisted);
            }
            let recap = totals.entry(host).or_insert_with(Recap::default);
            recap.ok += runner.recap.ok;
            recap.changed += runner.recap.changed;
            recap.failed += runner.recap.failed;
            recap.unreachable += runner.recap.unreachable
                + runner
                    .backend
                    .connection_failures()
                    .saturating_sub(connection_failures_before);
            recap.skipped += runner.recap.skipped;
            recap.rescued += runner.recap.rescued;
            recap.ignored += runner.recap.ignored;
            recap.filtered |= runner.filtered;
            match execution {
                // The session stopped before its tasks were all sent:
                // the agent never receives the rest and never calls
                // `abort_plan`, so the shutdown must not prune them.
                Err(error) => {
                    recap.aborted = true;
                    let error =
                        if layer.control_sensitive || runner.backend.conservative_sensitive() {
                            anyhow::anyhow!("sensitive execution failed")
                        } else {
                            error
                        };
                    return Err(RunFailure::new(error, totals).into());
                }
                // A task that stops the play without an execution
                // error — an agent result overridden by `failedWhen`,
                // or a controller-local fail — ends the session the
                // same way: the later tasks are never sent, so the
                // recap carries the aborted mark too.
                Ok(false) => {
                    recap.aborted = true;
                }
                Ok(true) => {}
            }
        }
    }
    Ok(totals)
}

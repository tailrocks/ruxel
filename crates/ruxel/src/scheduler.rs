//! The linear per-host scheduler (M3): walk the play's tasks in order,
//! render each one just-in-time with the full variable scope (play vars,
//! facts, registered results), evaluate when/loop per SEMANTICS §3,
//! execute — controller-side for assert/fail/debug/set_fact, on the agent
//! for everything else — then build the registered envelope with
//! task_eval and report ansible-shaped lines. Pipelined issue windows
//! (ARCHITECTURE §4) replace this walk once the ledger lands; observable
//! semantics stay identical.

use crate::transport::AgentConnection;
use anyhow::{Context, Result, anyhow, bail};
use minijinja::value::Value;
use ruxel_core::compiler::{PlanBody, PlanTask, PlayPlan, Readiness};
use ruxel_core::engine::{Engine, Scope, VarValue};
use ruxel_core::playbook::{Condition, Play, Task, TaskBody};
use ruxel_core::task_eval;
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
use std::collections::BTreeSet;
use std::io::Write;

/// Scheduler boundary for one fully rendered agent iteration.
#[allow(async_fn_in_trait)]
pub trait AgentExec {
    async fn run_batch(&mut self, tasks: Vec<v1::RenderedTask>, patch: bool) -> Result<Vec<Value>>;

    async fn run_iteration(&mut self, task: v1::RenderedTask) -> Result<Value> {
        self.run_batch(vec![task], false)
            .await?
            .into_iter()
            .next()
            .context("agent returned no result for iteration")
    }
}

impl AgentExec for AgentConnection {
    async fn run_batch(&mut self, tasks: Vec<v1::RenderedTask>, patch: bool) -> Result<Vec<Value>> {
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        let message = if patch {
            EnvMsg::PlanPatch(v1::PlanPatch {
                tasks: tasks.clone(),
            })
        } else {
            EnvMsg::Plan(v1::Plan {
                tasks: tasks.clone(),
                blobs_referenced: vec![],
            })
        };
        self.send(&v1::Envelope { msg: Some(message) }).await?;

        let mut results = Vec::with_capacity(tasks.len());
        loop {
            let event = self.next_event().await?.context("agent closed mid-batch")?;
            match event.msg {
                Some(EvMsg::TaskStart(_)) | Some(EvMsg::Log(_)) => continue,
                Some(EvMsg::TaskResult(res)) => {
                    let expected = tasks
                        .get(results.len())
                        .context("agent returned excess task result")?;
                    if res.task_id != expected.task_id {
                        bail!(
                            "agent batch result out of order: expected {}, got {}",
                            expected.task_id,
                            res.task_id
                        );
                    }
                    let json = if res.result_json.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_slice(&res.result_json)?
                    };
                    let failed = res.status == "failed"
                        || json.get("failed").and_then(|value| value.as_bool()) == Some(true);
                    results.push(to_mj(json));
                    if results.len() == tasks.len() || (failed && expected.halt_on_failure) {
                        return Ok(results);
                    }
                }
                Some(EvMsg::Crash(c)) => bail!("agent crashed: {} at {}", c.message, c.location),
                other => bail!("unexpected agent event mid-batch: {other:?}"),
            }
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recap {
    pub ok: u32,
    pub changed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub rescued: u32,
    pub ignored: u32,
}

/// Output shape (SEMANTICS §7): ansible-shaped human lines, or one stable
/// JSON object per task event on its own line (`--output json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

struct HostRun<'a, A, L> {
    engine: &'a Engine,
    agent: &'a mut A,
    event_log: &'a mut L,
    playbook_dir: std::path::PathBuf,
    host: String,
    play_vars: Vec<(String, VarValue)>,
    facts: Vec<(String, VarValue)>,
    registered: Vec<(String, VarValue)>,
    notified: BTreeSet<String>,
    recap: Recap,
    next_task_id: u64,
    format: OutputFormat,
    /// `--tags` selection (SEMANTICS §4): None = run everything; Some =
    /// run tasks whose effective tags intersect this set or include
    /// `always`. Block tags propagate to contained tasks.
    tags_filter: Option<Vec<String>>,
}

#[derive(Clone, Default)]
struct InheritedCtx {
    vars: Vec<(String, serde_norway::Value)>,
    environment: Vec<(String, serde_norway::Value)>,
    becomes: Option<bool>,
    become_user: Option<String>,
}

impl InheritedCtx {
    fn for_block(&self, task: &Task) -> Self {
        let mut next = self.clone();
        next.vars.extend(task.vars.iter().cloned());
        for (key, value) in &task.environment {
            if let Some(existing) = next.environment.iter_mut().find(|(name, _)| name == key) {
                existing.1 = value.clone();
            } else {
                next.environment.push((key.clone(), value.clone()));
            }
        }
        if task.becomes.is_some() {
            next.becomes = task.becomes;
        }
        if task.become_user.is_some() {
            next.become_user = task.become_user.clone();
        }
        next
    }
}

impl<A, L> HostRun<'_, A, L> {
    /// Whether a task runs under the active --tags filter, given the tags
    /// inherited from any enclosing block.
    fn tag_selected(&self, task: &Task, inherited: &[String]) -> bool {
        let Some(filter) = &self.tags_filter else {
            return true;
        };
        let effective = task.tags.iter().chain(inherited);
        effective
            .clone()
            .any(|t| t == "always" || filter.iter().any(|f| f == t))
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_play<A: AgentExec, L: Write>(
    play: &Play,
    compiled: &PlayPlan,
    host: &str,
    facts: &v1::Facts,
    engine: &Engine,
    agent: &mut A,
    playbook_dir: &std::path::Path,
    format: OutputFormat,
    tags_filter: Option<Vec<String>>,
    out: &mut impl Write,
    event_log: &mut L,
) -> Result<Recap> {
    let mut run = HostRun {
        engine,
        agent,
        event_log,
        playbook_dir: playbook_dir.to_path_buf(),
        host: host.to_string(),
        play_vars: play
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), VarValue::Raw(v.clone())))
            .collect(),
        facts: fact_layer(facts),
        registered: Vec::new(),
        notified: BTreeSet::new(),
        recap: Recap::default(),
        next_task_id: 1,
        format,
        tags_filter,
    };

    let mut host_failed = false;
    let inherited = InheritedCtx::default();
    'sections: for (section, compiled_section) in [
        (&play.pre_tasks, &compiled.pre_tasks),
        (&play.tasks, &compiled.tasks),
    ] {
        let mut index = 0;
        while index < section.len() {
            if let Some((consumed, failed)) = run
                .run_static_window(section, compiled_section, index, &inherited, out)
                .await?
            {
                index += consumed;
                if failed {
                    host_failed = true;
                    break 'sections;
                }
                continue;
            }
            let task = &section[index];
            let planned = &compiled_section[index];
            if run
                .run_task_or_block(task, planned, &[], &inherited, out)
                .await?
            {
                host_failed = true;
                break 'sections;
            }
            index += 1;
        }
    }

    // Handlers flush at end of play, definition order, once each, only if
    // notified by a changed task (SEMANTICS §4).
    if !host_failed {
        for (handler, planned) in play.handlers.iter().zip(&compiled.handlers) {
            let name = handler.name.clone().unwrap_or_default();
            if run.notified.contains(&name)
                && run
                    .run_task_or_block(handler, planned, &[], &inherited, out)
                    .await?
            {
                break; // handler failure ends the play; recap already counted
            }
        }
    }

    Ok(run.recap)
}

fn fact_layer(facts: &v1::Facts) -> Vec<(String, VarValue)> {
    let j = serde_json::json!({
        "ansible_default_ipv4": {"interface": facts.default_ipv4_interface},
        "ansible_facts": {"distribution_release": facts.distribution_release},
        "ansible_architecture": facts.architecture,
        "ansible_hostname": facts.hostname,
    });
    j.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), VarValue::Final(Value::from_serialize(v))))
        .collect()
}

impl<A: AgentExec, L: Write> HostRun<'_, A, L> {
    fn scope(
        &self,
        inherited: &InheritedCtx,
        task_vars: &[(String, serde_norway::Value)],
    ) -> Scope {
        let mut scope = Scope::new()
            .with_layer(self.play_vars.clone())
            .with_layer(self.facts.clone())
            .with_layer(self.registered.clone());
        if !inherited.vars.is_empty() {
            scope = scope.with_layer(
                inherited
                    .vars
                    .iter()
                    .map(|(k, v)| (k.clone(), VarValue::Raw(v.clone())))
                    .collect(),
            );
        }
        if !task_vars.is_empty() {
            scope = scope.with_layer(
                task_vars
                    .iter()
                    .map(|(k, v)| (k.clone(), VarValue::Raw(v.clone())))
                    .collect(),
            );
        }
        scope
    }

    fn register(&mut self, name: &str, value: Value) {
        self.registered
            .push((name.to_string(), VarValue::Final(value)));
    }

    /// Dispatch one maximal dependency-free issue window. Complex task
    /// controls remain sequential; compiler-static plain agent tasks can be
    /// rendered up front and drained by the agent in one wire Plan.
    async fn run_static_window(
        &mut self,
        tasks: &[Task],
        planned: &[PlanTask],
        start: usize,
        inherited: &InheritedCtx,
        out: &mut impl Write,
    ) -> Result<Option<(usize, bool)>> {
        let mut end = start;
        while end < tasks.len()
            && self.tag_selected(&tasks[end], &[])
            && static_batch_candidate(&tasks[end], &planned[end])
        {
            end += 1;
        }
        if end == start {
            return Ok(None);
        }

        let mut rendered = Vec::with_capacity(end - start);
        for index in start..end {
            rendered.push(self.prepare_static_task(&tasks[index], &planned[index], inherited)?);
        }
        let results = self.agent.run_batch(rendered, false).await?;
        if results.is_empty() {
            bail!("agent returned no results for static issue window");
        }
        let mut failed = false;
        for (offset, result) in results.into_iter().enumerate() {
            failed = self.finalize_result(&tasks[start + offset], result, out);
            if failed {
                break;
            }
        }
        // A halted batch consumes every sent task from this host's schedule:
        // later tasks were deliberately not executed and host processing ends.
        Ok(Some((end - start, failed)))
    }

    fn prepare_static_task(
        &mut self,
        task: &Task,
        planned: &PlanTask,
        inherited: &InheritedCtx,
    ) -> Result<v1::RenderedTask> {
        let TaskBody::Module(call) = &task.body else {
            unreachable!("static window excludes blocks")
        };
        let PlanBody::Module {
            readiness:
                Readiness::Static {
                    params,
                    free_form,
                    loop_items: None,
                },
            ..
        } = &planned.body
        else {
            unreachable!("static window requires static module")
        };
        let scope = self.scope(inherited, &task.vars);
        let mut params_json = params
            .iter()
            .map(|(key, value)| Ok((key.clone(), serde_json::to_value(value)?)))
            .collect::<Result<serde_json::Map<String, serde_json::Value>>>()?;
        ruxel_core::compiler::validate_rendered_enums(
            call.module,
            params,
            &self.playbook_dir.to_string_lossy(),
            &label(task),
        )?;
        let module = call.module.name;
        if (module == "copy" || module == "template")
            && !params_json.contains_key("content")
            && let Some(src) = params_json.get("src").and_then(|value| value.as_str())
        {
            let path = self.playbook_dir.join(src);
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| anyhow!("{module} src {}: {error}", path.display()))?;
            let content = if module == "template" {
                self.engine.render_template_file(&raw, &scope)?
            } else {
                raw
            };
            params_json.remove("src");
            params_json.insert("content".into(), serde_json::Value::String(content));
        }
        let params_bytes = serde_json::to_vec(&params_json)?;
        let free_form = free_form.clone().unwrap_or_default();
        let environment = self.render_environment(task, inherited, &scope)?;
        let task_id = self.next_task_id;
        self.next_task_id += 1;
        let ledger_key = ledger_key(
            &self.playbook_dir,
            module,
            &label(task),
            "",
            &params_bytes,
            &free_form,
        );
        Ok(v1::RenderedTask {
            task_id,
            name: label(task),
            module: module.to_string(),
            rendered: true,
            iterations: vec![v1::Iteration {
                item_label: String::new(),
                params_json: params_bytes,
                free_form,
                ledger_key,
            }],
            check_mode_override: task.check_mode == Some(false),
            no_log: task.no_log,
            become_user: if task.becomes.or(inherited.becomes) == Some(false) {
                String::new()
            } else {
                task.become_user
                    .clone()
                    .or_else(|| inherited.become_user.clone())
                    .unwrap_or_default()
            },
            environment,
            halt_on_failure: !task.ignore_errors,
        })
    }

    fn render_environment(
        &self,
        task: &Task,
        inherited: &InheritedCtx,
        scope: &Scope,
    ) -> Result<std::collections::HashMap<String, String>> {
        let mut environment = std::collections::HashMap::new();
        for (key, value) in inherited.environment.iter().chain(&task.environment) {
            let rendered = self.engine.render_value(value, scope)?;
            environment.insert(
                key.clone(),
                rendered
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| rendered.to_string()),
            );
        }
        Ok(environment)
    }

    /// Returns true when the host must stop (unrescued failure).
    /// `inherited_tags` are the tags of any enclosing block (SEMANTICS §4).
    async fn run_task_or_block(
        &mut self,
        task: &Task,
        planned: &PlanTask,
        inherited_tags: &[String],
        inherited: &InheritedCtx,
        out: &mut impl Write,
    ) -> Result<bool> {
        if let TaskBody::Block {
            block,
            rescue,
            always,
        } = &task.body
        {
            let PlanBody::Block {
                block: planned_block,
                rescue: planned_rescue,
                always: planned_always,
            } = &planned.body
            else {
                bail!("compiled/source task shape mismatch for {}", label(task));
            };
            // The block's own tags propagate to its contained tasks.
            let mut child_tags: Vec<String> = inherited_tags.to_vec();
            child_tags.extend(task.tags.iter().cloned());

            // Block-level when gates the whole block (SEMANTICS §4).
            if let Some(when) = &task.when {
                let scope = self.scope(inherited, &task.vars);
                if !self.engine.eval_condition(when, &scope)? {
                    for sub in block {
                        self.print_status(out, sub, "skipped", None);
                        self.recap.skipped += 1;
                    }
                    return Ok(false);
                }
            }
            let child_ctx = inherited.for_block(task);
            let mut block_failed = false;
            for (sub, planned_sub) in block.iter().zip(planned_block) {
                if Box::pin(self.run_task_or_block(sub, planned_sub, &child_tags, &child_ctx, out))
                    .await?
                {
                    block_failed = true;
                    break;
                }
            }
            let mut host_failed = false;
            if block_failed {
                if rescue.is_empty() {
                    host_failed = true;
                } else {
                    self.recap.rescued += 1;
                    for (sub, planned_sub) in rescue.iter().zip(planned_rescue) {
                        if Box::pin(self.run_task_or_block(
                            sub,
                            planned_sub,
                            &child_tags,
                            &child_ctx,
                            out,
                        ))
                        .await?
                        {
                            host_failed = true;
                            break;
                        }
                    }
                }
            }
            for (sub, planned_sub) in always.iter().zip(planned_always) {
                if Box::pin(self.run_task_or_block(sub, planned_sub, &child_tags, &child_ctx, out))
                    .await?
                {
                    host_failed = true;
                    break;
                }
            }
            return Ok(host_failed);
        }

        // --tags: an unselected task reports skipped and does not run.
        if !self.tag_selected(task, inherited_tags) {
            self.print_status(out, task, "skipped", None);
            self.recap.skipped += 1;
            return Ok(false);
        }

        self.run_module_task(task, planned, inherited, out).await
    }

    async fn run_module_task(
        &mut self,
        task: &Task,
        planned: &PlanTask,
        inherited: &InheritedCtx,
        out: &mut impl Write,
    ) -> Result<bool> {
        let TaskBody::Module(call) = &task.body else {
            unreachable!("blocks handled by caller")
        };
        let PlanBody::Module { readiness, .. } = &planned.body else {
            bail!("compiled/source task shape mismatch for {}", label(task));
        };
        let scope = self.scope(inherited, &task.vars);

        // Loop expansion (per-item when) or single-shot when.
        let loop_items: Option<Vec<Value>> = match &task.loop_ {
            None => None,
            Some(v) => {
                let rendered = self
                    .engine
                    .render_value(v, &scope)
                    .map_err(|e| anyhow!("{}: loop: {e}", label(task)))?;
                Some(
                    rendered
                        .try_iter()
                        .map_err(|e| anyhow!("loop: {e}"))?
                        .collect(),
                )
            }
        };

        let result = match loop_items {
            None => {
                if let Some(when) = &task.when {
                    let outcomes = eval_when_parts(self.engine, when, &scope)?;
                    if outcomes.iter().any(|ok| !ok) {
                        let fc = task_eval::first_false_condition(when, &outcomes);
                        let skip = task_eval::skipped_result(&fc);
                        if let Some(reg) = &task.register {
                            self.register(reg, skip.clone());
                        }
                        self.finish_task(task, out, skip, "skipped", false);
                        return Ok(false);
                    }
                }
                self.execute_iterations(
                    task,
                    call,
                    readiness,
                    vec![(None, scope.clone())],
                    inherited,
                    out,
                )
                .await?
            }
            Some(items) => {
                if items.is_empty() {
                    let agg = task_eval::loop_aggregate(vec![]);
                    if let Some(reg) = &task.register {
                        self.register(reg, agg.clone());
                    }
                    self.finish_task(task, out, agg, "skipped", false);
                    return Ok(false);
                }
                let mut iterations: Vec<(Option<Value>, Scope)> = Vec::new();
                for item in items {
                    let item_scope =
                        scope.with_layer(vec![("item".to_string(), VarValue::Final(item.clone()))]);
                    iterations.push((Some(item), item_scope));
                }
                self.execute_iterations(task, call, readiness, iterations, inherited, out)
                    .await?
            }
        };

        Ok(self.finalize_result(task, result, out))
    }

    fn finalize_result(&mut self, task: &Task, result: Value, out: &mut impl Write) -> bool {
        let failed = result_failed(&result);
        let changed = result_truthy(&result, "changed");
        let skipped = result_truthy(&result, "skipped");

        // notify on final changed (SEMANTICS §3.12).
        if changed && !failed {
            for h in &task.notify {
                self.notified.insert(h.clone());
            }
        }
        if let Some(reg) = &task.register {
            self.register(reg, result.clone());
        }

        let status = if failed {
            "failed"
        } else if skipped {
            "skipped"
        } else if changed {
            "changed"
        } else {
            "ok"
        };
        self.finish_task(task, out, result, status, failed && task.ignore_errors);
        failed && !task.ignore_errors
    }

    /// Execute the task's iterations (single or per-item), including
    /// controller-side modules, until/retries, and per-item when handled
    /// by the caller for loops.
    async fn execute_iterations(
        &mut self,
        task: &Task,
        call: &ruxel_core::playbook::ModuleCall,
        readiness: &Readiness,
        iterations: Vec<(Option<Value>, Scope)>,
        inherited: &InheritedCtx,
        _out: &mut impl Write,
    ) -> Result<Value> {
        let is_loop = iterations.len() != 1 || iterations[0].0.is_some();
        let mut per_item: Vec<Value> = Vec::new();

        for (item, item_scope) in iterations {
            // Per-item when (loops only — single-shot handled by caller).
            if item.is_some()
                && let Some(when) = &task.when
            {
                let outcomes = eval_when_parts(self.engine, when, &item_scope)?;
                if outcomes.iter().any(|ok| !ok) {
                    let fc = task_eval::first_false_condition(when, &outcomes);
                    let skip = task_eval::skipped_result(&fc);
                    per_item.push(task_eval::decorate_loop_item(&skip, item.as_ref().unwrap()));
                    continue;
                }
            }

            let mut attempts: u64 = 0;
            let max_attempts = task.retries.map(|r| r + 1).unwrap_or(1);
            let raw = loop {
                attempts += 1;
                let raw = self
                    .execute_once(task, call, readiness, &item_scope, item.as_ref(), inherited)
                    .await?;
                let Some(until) = &task.until else { break raw };
                // The until expression sees the candidate result under the
                // register name (SEMANTICS §3.10).
                let mut cand_scope = item_scope.clone();
                if let Some(reg) = &task.register {
                    cand_scope =
                        cand_scope.with_layer(vec![(reg.clone(), VarValue::Final(raw.clone()))]);
                }
                if self.engine.eval_condition(until, &cand_scope)? {
                    break raw;
                }
                if attempts >= max_attempts {
                    break merge_failed(&raw);
                }
                tokio::time::sleep(std::time::Duration::from_secs(task.delay.unwrap_or(5))).await;
            };
            let raw = if task.until.is_some() {
                task_eval::finalize_until(&raw, attempts)
            } else {
                raw
            };

            // changed_when / failed_when see the raw result (+item).
            let mut decorated = raw;
            let mut eval_scope = item_scope.clone();
            if let Some(reg) = &task.register {
                eval_scope =
                    eval_scope.with_layer(vec![(reg.clone(), VarValue::Final(decorated.clone()))]);
            }
            if let Some(fw) = &task.failed_when {
                let outcome = self.engine.eval_condition(fw, &eval_scope)?;
                decorated = set_key(&decorated, "failed", Value::from(outcome));
                decorated = set_key(&decorated, "failed_when_result", Value::from(outcome));
            }
            if let Some(cw) = &task.changed_when {
                let outcome = self.engine.eval_condition(cw, &eval_scope)?;
                decorated = task_eval::apply_changed_when(&decorated, outcome);
            }

            per_item.push(match &item {
                Some(it) => task_eval::decorate_loop_item(&decorated, it),
                None => decorated,
            });
        }

        Ok(if is_loop {
            task_eval::loop_aggregate(per_item)
        } else {
            per_item.into_iter().next().expect("one iteration")
        })
    }

    async fn execute_once(
        &mut self,
        task: &Task,
        call: &ruxel_core::playbook::ModuleCall,
        readiness: &Readiness,
        scope: &Scope,
        item: Option<&Value>,
        inherited: &InheritedCtx,
    ) -> Result<Value> {
        let module = call.module.name;
        // Controller-side modules: no agent round-trip (ARCHITECTURE §4).
        match module {
            "debug" => {
                let msg = match call.params.iter().find(|(k, _)| k == "msg") {
                    Some((_, v)) => self.engine.render_value(v, scope)?,
                    None => Value::from("Hello world!"),
                };
                return Ok(serde_json::to_value(&msg)
                    .map(|m| serde_json::json!({"msg": m, "changed": false, "failed": false}))
                    .map(to_mj)?);
            }
            "set_fact" => {
                let mut set = serde_json::Map::new();
                for (k, v) in &call.params {
                    let rendered = self.engine.render_value(v, scope)?;
                    self.register(k, rendered.clone());
                    set.insert(k.clone(), serde_json::to_value(&rendered)?);
                }
                return Ok(to_mj(
                    serde_json::json!({"ansible_facts": set, "changed": false, "failed": false}),
                ));
            }
            "fail" => {
                let msg = match call.params.iter().find(|(k, _)| k == "msg") {
                    Some((_, v)) => self.engine.render_value(v, scope)?,
                    None => Value::from("Failed as requested from task"),
                };
                return Ok(to_mj(serde_json::json!({
                    "failed": true, "changed": false,
                    "msg": serde_json::to_value(&msg)?,
                })));
            }
            "assert" => {
                let that = call
                    .params
                    .iter()
                    .find(|(k, _)| k == "that")
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| anyhow!("assert: that required"))?;
                let exprs: Vec<String> = match that {
                    serde_norway::Value::String(s) => vec![s],
                    serde_norway::Value::Sequence(items) => items
                        .into_iter()
                        .map(|v| match v {
                            serde_norway::Value::String(s) => Ok(s),
                            other => Err(anyhow!("assert.that entries must be strings: {other:?}")),
                        })
                        .collect::<Result<_>>()?,
                    other => bail!("assert.that must be a string or list: {other:?}"),
                };
                for expr in &exprs {
                    let ok = self
                        .engine
                        .eval_condition(&Condition::Expr(expr.clone()), scope)?;
                    if !ok {
                        let fail_msg = match call.params.iter().find(|(k, _)| k == "fail_msg") {
                            Some((_, v)) => {
                                serde_json::to_value(&self.engine.render_value(v, scope)?)?
                            }
                            None => serde_json::Value::String(format!("Assertion failed: {expr}")),
                        };
                        return Ok(to_mj(serde_json::json!({
                            "failed": true, "changed": false,
                            "assertion": expr, "evaluated_to": false,
                            "msg": fail_msg,
                        })));
                    }
                }
                return Ok(to_mj(serde_json::json!({
                    "failed": false, "changed": false,
                    "msg": "All assertions passed",
                })));
            }
            "pause" => {
                // Interactive pause (SEMANTICS §4): controller-side, prompt
                // on the operator TTY and block for Enter. When stdin is not
                // a TTY (CI, --check), Ansible still prompts but a closed
                // stdin returns immediately; mirror that — print and proceed.
                let prompt = match call.params.iter().find(|(k, _)| k == "prompt") {
                    Some((_, v)) => self
                        .engine
                        .render_value(v, scope)?
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_default(),
                    None => "Press enter to continue, Ctrl+C to abort".to_string(),
                };
                use std::io::IsTerminal as _;
                eprint!("[pause] {prompt}\n[pause] press Enter to continue: ");
                let _ = std::io::Write::flush(&mut std::io::stderr());
                if std::io::stdin().is_terminal() {
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                }
                eprintln!();
                return Ok(to_mj(serde_json::json!({
                    "changed": false, "failed": false,
                    "user_input": "",
                })));
            }
            _ => {}
        }

        // Agent-side execution: render params + free-form with the item
        // scope, ship one iteration, await its result.
        let (mut params, rendered_params, compiled_free_form) = if item.is_none()
            && let Readiness::Static {
                params,
                free_form,
                loop_items: None,
            } = readiness
        {
            let json = params
                .iter()
                .map(|(key, value)| Ok((key.clone(), serde_json::to_value(value)?)))
                .collect::<Result<serde_json::Map<String, serde_json::Value>>>()?;
            (
                json,
                params.clone(),
                Some(free_form.clone().unwrap_or_default()),
            )
        } else {
            let mut json = serde_json::Map::new();
            let mut rendered = Vec::new();
            for (key, value) in &call.params {
                let value = self.engine.render_value(value, scope)?;
                json.insert(key.clone(), serde_json::to_value(&value)?);
                rendered.push((key.clone(), value));
            }
            (json, rendered, None)
        };
        ruxel_core::compiler::validate_rendered_enums(
            call.module,
            &rendered_params,
            &self.playbook_dir.to_string_lossy(),
            &label(task),
        )?;
        // `copy src=` reads the controller-side file (playbook-relative)
        // and ships it as content; `template src=` additionally renders
        // it through the engine with the full scope first (byte-fidelity
        // proven by the M1 render-parity gate). All file payload stays a
        // controller concern (ARCHITECTURE §1); the content-addressed
        // blob channel later replaces inline shipping, not this logic.
        if (module == "copy" || module == "template")
            && !params.contains_key("content")
            && let Some(src) = params.get("src").and_then(|v| v.as_str())
        {
            let path = self.playbook_dir.join(src);
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| anyhow!("{module} src {}: {e}", path.display()))?;
            let content = if module == "template" {
                self.engine.render_template_file(&raw, scope)?
            } else {
                raw
            };
            params.remove("src");
            params.insert("content".into(), serde_json::Value::String(content));
        }
        let free_form = match compiled_free_form {
            Some(rendered) => rendered,
            None => match &call.free_form {
                Some(body) => self
                    .engine
                    .render_str(body, scope)?
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_default(),
                None => String::new(),
            },
        };
        let item_label = item
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| i.to_string())
            })
            .unwrap_or_default();

        let task_id = self.next_task_id;
        self.next_task_id += 1;
        let environment: std::collections::HashMap<String, String> = {
            let mut env = std::collections::HashMap::new();
            for (k, v) in &inherited.environment {
                let rendered = self.engine.render_value(v, scope)?;
                env.insert(
                    k.clone(),
                    rendered
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| rendered.to_string()),
                );
            }
            for (k, v) in &task.environment {
                let rendered = self.engine.render_value(v, scope)?;
                env.insert(
                    k.clone(),
                    rendered
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| rendered.to_string()),
                );
            }
            env
        };

        // Stable ledger identity (ARCHITECTURE §6): blake3 over the task's
        // identity and its fully-rendered params/body/item. Any change to
        // params (including a rotated secret, which renders to different
        // bytes) yields a different key → cache miss → re-check. A hash, so
        // no secret is recoverable from it.
        let params_bytes = serde_json::to_vec(&params)?;
        let ledger_key = {
            let mut h = blake3::Hasher::new();
            h.update(self.playbook_dir.to_string_lossy().as_bytes());
            h.update(b"\x1f");
            h.update(module.as_bytes());
            h.update(b"\x1f");
            h.update(label(task).as_bytes());
            h.update(b"\x1f");
            h.update(item_label.as_bytes());
            h.update(b"\x1f");
            h.update(&params_bytes);
            h.update(b"\x1f");
            h.update(free_form.as_bytes());
            h.finalize().to_hex().to_string()
        };

        let rendered_task = v1::RenderedTask {
            task_id,
            name: label(task),
            module: module.to_string(),
            rendered: true,
            iterations: vec![v1::Iteration {
                item_label,
                params_json: params_bytes,
                free_form,
                ledger_key,
            }],
            check_mode_override: task.check_mode == Some(false),
            no_log: task.no_log,
            become_user: if task.becomes.or(inherited.becomes) == Some(false) {
                String::new()
            } else {
                task.become_user
                    .clone()
                    .or_else(|| inherited.become_user.clone())
                    .unwrap_or_default()
            },
            environment,
            halt_on_failure: !task.ignore_errors,
        };
        self.agent
            .run_batch(
                vec![rendered_task],
                matches!(readiness, Readiness::Deferred { .. }),
            )
            .await?
            .into_iter()
            .next()
            .context("agent returned no result for iteration")
    }

    /// Recap accounting mirrors Ansible's: `ok` includes changed tasks;
    /// an ignored failure counts only under `ignored`.
    fn finish_task(
        &mut self,
        task: &Task,
        out: &mut impl Write,
        result: Value,
        status: &str,
        ignored: bool,
    ) {
        match status {
            "failed" if ignored => self.recap.ignored += 1,
            "failed" => self.recap.failed += 1,
            "skipped" => self.recap.skipped += 1,
            "changed" => {
                self.recap.changed += 1;
                self.recap.ok += 1;
            }
            _ => self.recap.ok += 1,
        }
        let display = if task.no_log {
            serde_json::to_string(&task_eval::censored_result(status == "changed", None))
                .unwrap_or_default()
        } else if status == "failed" {
            serde_json::to_string(&result).unwrap_or_default()
        } else {
            String::new()
        };
        self.print_status(out, task, status, None);
        if !display.is_empty() {
            let _ = writeln!(out, "    {display}");
        }
        // --diff: print the content diff the agent produced (no_log-safe:
        // redacted tasks never carry one).
        if self.format == OutputFormat::Human
            && !task.no_log
            && let Some(diff) = result.get_attr("diff").ok().filter(|d| !d.is_undefined())
            && let Some(diff) = diff.as_str()
            && !diff.is_empty()
        {
            let _ = writeln!(out, "{diff}");
        }
    }

    fn print_status(
        &mut self,
        out: &mut impl Write,
        task: &Task,
        status: &str,
        item: Option<&str>,
    ) {
        let event = serde_json::json!({
            "event": "task",
            "host": self.host,
            "task": label(task),
            "status": status,
            "item": item,
        });
        let _ = writeln!(self.event_log, "{event}");
        match self.format {
            OutputFormat::Human => {
                let _ = writeln!(out, "TASK [{}] {}", label(task), "*".repeat(20));
                match item {
                    Some(i) => {
                        let _ = writeln!(out, "{status}: [{}] => (item={i})", self.host);
                    }
                    None => {
                        let _ = writeln!(out, "{status}: [{}]", self.host);
                    }
                }
            }
            OutputFormat::Json => {
                let _ = writeln!(out, "{event}");
            }
        }
    }
}

fn label(task: &Task) -> String {
    task.name.clone().unwrap_or_else(|| "(unnamed)".into())
}

fn static_batch_candidate(task: &Task, planned: &PlanTask) -> bool {
    let TaskBody::Module(call) = &task.body else {
        return false;
    };
    let PlanBody::Module {
        readiness: Readiness::Static {
            loop_items: None, ..
        },
        ..
    } = &planned.body
    else {
        return false;
    };
    !matches!(
        call.module.name,
        "assert" | "debug" | "fail" | "pause" | "set_fact"
    ) && task.loop_.is_none()
        && task.when.is_none()
        && task.until.is_none()
        && task.retries.is_none()
        && task.delay.is_none()
        && task.changed_when.is_none()
        && task.failed_when.is_none()
}

fn ledger_key(
    playbook_dir: &std::path::Path,
    module: &str,
    task_label: &str,
    item_label: &str,
    params: &[u8],
    free_form: &str,
) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(playbook_dir.to_string_lossy().as_bytes());
    hash.update(b"\x1f");
    hash.update(module.as_bytes());
    hash.update(b"\x1f");
    hash.update(task_label.as_bytes());
    hash.update(b"\x1f");
    hash.update(item_label.as_bytes());
    hash.update(b"\x1f");
    hash.update(params);
    hash.update(b"\x1f");
    hash.update(free_form.as_bytes());
    hash.finalize().to_hex().to_string()
}

fn eval_when_parts(engine: &Engine, when: &Condition, scope: &Scope) -> Result<Vec<bool>> {
    Ok(match when {
        Condition::Literal(b) => vec![*b],
        Condition::Expr(e) => vec![engine.eval_condition(&Condition::Expr(e.clone()), scope)?],
        Condition::All(exprs) => {
            let mut outcomes = Vec::with_capacity(exprs.len());
            for e in exprs {
                let ok = engine.eval_condition(&Condition::Expr(e.clone()), scope)?;
                outcomes.push(ok);
                if !ok {
                    break; // short-circuit AND, like Ansible
                }
            }
            outcomes
        }
    })
}

fn to_mj(j: serde_json::Value) -> Value {
    Value::from_serialize(&j)
}

fn set_key(map: &Value, key: &str, value: Value) -> Value {
    let mut pairs: Vec<(Value, Value)> = Vec::new();
    if let Ok(iter) = map.try_iter() {
        for k in iter {
            if k.as_str() != Some(key)
                && let Ok(v) = map.get_item(&k)
            {
                pairs.push((k, v));
            }
        }
    }
    pairs.push((Value::from(key), value));
    Value::from_iter(pairs)
}

fn merge_failed(result: &Value) -> Value {
    set_key(result, "failed", Value::from(true))
}

fn result_truthy(result: &Value, key: &str) -> bool {
    result
        .get_attr(key)
        .map(|v| !v.is_undefined() && v.is_true())
        .unwrap_or(false)
}

fn result_failed(result: &Value) -> bool {
    result_truthy(result, "failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruxel_core::engine::{DrySecrets, MemoizedResolver};
    use std::collections::VecDeque;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeAgent {
        scripted: VecDeque<serde_json::Value>,
        calls: Vec<v1::RenderedTask>,
        batches: Vec<Vec<u64>>,
        patches: Vec<bool>,
    }

    impl FakeAgent {
        fn with_results(results: impl IntoIterator<Item = serde_json::Value>) -> Self {
            Self {
                scripted: results.into_iter().collect(),
                calls: Vec::new(),
                batches: Vec::new(),
                patches: Vec::new(),
            }
        }
    }

    impl AgentExec for FakeAgent {
        async fn run_batch(
            &mut self,
            tasks: Vec<v1::RenderedTask>,
            patch: bool,
        ) -> Result<Vec<Value>> {
            self.batches
                .push(tasks.iter().map(|task| task.task_id).collect());
            self.patches.push(patch);
            let mut results = Vec::new();
            for task in tasks {
                let result = self
                    .scripted
                    .pop_front()
                    .unwrap_or_else(|| serde_json::json!({"changed": true, "failed": false}));
                let failed = result.get("failed").and_then(|value| value.as_bool()) == Some(true);
                let halt = task.halt_on_failure;
                self.calls.push(task);
                results.push(to_mj(result));
                if failed && halt {
                    break;
                }
            }
            Ok(results)
        }
    }

    async fn run(
        yaml: &str,
        results: impl IntoIterator<Item = serde_json::Value>,
    ) -> (Recap, FakeAgent, String) {
        let playbook = ruxel_core::playbook::parse("test.yml", yaml).unwrap();
        let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
        let compiled = ruxel_core::compiler::compile(&playbook, &engine).unwrap();
        let mut agent = FakeAgent::with_results(results);
        let mut output = Vec::new();
        let mut event_log = Vec::new();
        let recap = run_play(
            &playbook.plays[0],
            &compiled.plays[0],
            "test-host",
            &v1::Facts::default(),
            &engine,
            &mut agent,
            std::path::Path::new("."),
            OutputFormat::Human,
            None,
            &mut output,
            &mut event_log,
        )
        .await
        .unwrap();
        (recap, agent, String::from_utf8(output).unwrap())
    }

    #[tokio::test]
    async fn command_changed_updates_recap() {
        let (recap, agent, _) = run(
            "- hosts: all\n  tasks:\n    - name: run\n      command: echo hi\n",
            [],
        )
        .await;
        assert_eq!(recap.ok, 1);
        assert_eq!(recap.changed, 1);
        assert_eq!(agent.calls.len(), 1);
    }

    #[tokio::test]
    async fn consecutive_static_tasks_share_one_issue_window() {
        let yaml = "- hosts: all\n  tasks:\n    - name: one\n      command: echo one\n    - name: two\n      command: echo two\n    - name: three\n      command: echo three\n";
        let (recap, agent, _) = run(yaml, []).await;
        assert_eq!(recap.ok, 3);
        assert_eq!(recap.changed, 3);
        assert_eq!(agent.batches, vec![vec![1, 2, 3]]);
        assert_eq!(
            agent
                .calls
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );
    }

    #[tokio::test]
    async fn failed_static_task_halts_remaining_issue_window() {
        let yaml = "- hosts: all\n  tasks:\n    - name: one\n      command: echo one\n    - name: two\n      command: 'false'\n    - name: three\n      command: echo three\n";
        let results = [
            serde_json::json!({"changed": false, "failed": false}),
            serde_json::json!({"changed": false, "failed": true}),
            serde_json::json!({"changed": false, "failed": false}),
        ];
        let (recap, agent, _) = run(yaml, results).await;
        assert_eq!(recap.ok, 1);
        assert_eq!(recap.failed, 1);
        assert_eq!(agent.batches, vec![vec![1, 2, 3]]);
        assert_eq!(
            agent
                .calls
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
    }

    #[tokio::test]
    async fn deferred_task_uses_register_and_streams_plan_patch() {
        let yaml = "- hosts: all\n  tasks:\n    - name: probe\n      stat:\n        path: /tmp/example\n      register: probe\n    - name: consume\n      command: echo {{ probe.stat.exists }}\n";
        let results = [
            serde_json::json!({
                "changed": false,
                "failed": false,
                "stat": {"exists": true}
            }),
            serde_json::json!({"changed": true, "failed": false}),
        ];
        let (recap, agent, _) = run(yaml, results).await;
        assert_eq!(recap.ok, 2);
        assert_eq!(agent.patches, vec![false, true]);
        assert_eq!(agent.calls[1].iterations[0].free_form, "echo True");
    }

    #[tokio::test]
    async fn no_log_result_payload_never_enters_event_log() {
        let yaml = "- hosts: all\n  tasks:\n    - name: sensitive task\n      command: echo hidden\n      no_log: true\n";
        let playbook = ruxel_core::playbook::parse("test.yml", yaml).unwrap();
        let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
        let compiled = ruxel_core::compiler::compile(&playbook, &engine).unwrap();
        let mut agent = FakeAgent::with_results([serde_json::json!({
            "changed": true,
            "failed": false,
            "stdout": "synthetic-secret-value"
        })]);
        let mut output = Vec::new();
        let mut event_log = Vec::new();
        run_play(
            &playbook.plays[0],
            &compiled.plays[0],
            "test-host",
            &v1::Facts::default(),
            &engine,
            &mut agent,
            std::path::Path::new("."),
            OutputFormat::Human,
            None,
            &mut output,
            &mut event_log,
        )
        .await
        .unwrap();
        let log = String::from_utf8(event_log).unwrap();
        assert!(log.contains("\"status\":\"changed\""));
        assert!(!log.contains("synthetic-secret-value"));
        assert!(
            !String::from_utf8(output)
                .unwrap()
                .contains("synthetic-secret-value")
        );
    }

    #[tokio::test]
    async fn false_when_skips_without_agent_call() {
        let (recap, agent, _) = run(
            "- hosts: all\n  tasks:\n    - name: skip\n      command: echo hi\n      when: false\n",
            [],
        )
        .await;
        assert_eq!(recap.skipped, 1);
        assert!(agent.calls.is_empty());
    }

    #[tokio::test]
    async fn loop_when_skips_only_false_item() {
        let (recap, agent, _) = run(
            "- hosts: all\n  tasks:\n    - name: loop\n      command: echo hi\n      loop: [1, 2]\n      when: item == 2\n",
            [],
        )
        .await;
        assert_eq!(recap.changed, 1);
        assert_eq!(agent.calls.len(), 1);
        assert_eq!(agent.calls[0].iterations[0].item_label, "2");
    }

    #[tokio::test]
    async fn changed_when_false_overrides_agent_result() {
        let (recap, _, _) = run(
            "- hosts: all\n  tasks:\n    - name: stable\n      command: echo hi\n      changed_when: false\n",
            [],
        )
        .await;
        assert_eq!(recap.ok, 1);
        assert_eq!(recap.changed, 0);
    }

    #[tokio::test]
    async fn rescue_continues_after_block_failure() {
        let yaml = "- hosts: all\n  tasks:\n    - name: guarded\n      block:\n        - name: fail inside\n          command: 'false'\n      rescue:\n        - name: recover\n          command: 'true'\n    - name: after\n      command: 'true'\n";
        let results = [
            serde_json::json!({"changed": false, "failed": true}),
            serde_json::json!({"changed": false, "failed": false}),
            serde_json::json!({"changed": false, "failed": false}),
        ];
        let (recap, agent, _) = run(yaml, results).await;
        assert_eq!(recap.rescued, 1);
        assert_eq!(agent.calls.len(), 3, "task after rescued block must run");
    }

    #[tokio::test]
    async fn notified_handler_runs_at_play_end() {
        let yaml = "- hosts: all\n  tasks:\n    - name: change\n      command: echo hi\n      notify: restart service\n  handlers:\n    - name: restart service\n      command: echo restart\n";
        let (recap, agent, _) = run(yaml, []).await;
        assert_eq!(recap.ok, 2);
        assert_eq!(recap.changed, 2);
        assert_eq!(agent.calls.len(), 2);
        assert_eq!(agent.calls[1].name, "restart service");
    }

    #[tokio::test]
    async fn skipped_single_task_registers_skip_dict() {
        let yaml = "- hosts: all\n  tasks:\n    - name: skipped\n      command: echo hi\n      when: false\n      register: r\n    - name: verify\n      assert:\n        that:\n          - r.skipped\n";
        let (recap, agent, _) = run(yaml, []).await;
        assert_eq!(recap.skipped, 1);
        assert_eq!(recap.ok, 1);
        assert!(agent.calls.is_empty());
    }

    #[tokio::test]
    async fn empty_loop_registers_skipped_empty_aggregate() {
        let yaml = "- hosts: all\n  tasks:\n    - name: empty\n      command: echo hi\n      loop: []\n      register: r\n    - name: verify\n      assert:\n        that:\n          - r.skipped\n          - r.results | length == 0\n";
        let (recap, agent, _) = run(yaml, []).await;
        assert_eq!(recap.skipped, 1);
        assert_eq!(recap.ok, 1);
        assert!(agent.calls.is_empty());
    }

    #[tokio::test]
    async fn block_context_is_inherited_with_task_precedence() {
        let yaml = "- hosts: all\n  tasks:\n    - name: contextual\n      become: true\n      become_user: inherited-user\n      vars:\n        inherited_var: inherited-value\n      environment:\n        INHERITED: '{{ inherited_var }}'\n        OVERRIDE: block-value\n      block:\n        - name: child\n          command: echo {{ inherited_var }}\n          environment:\n            OVERRIDE: task-value\n";
        let (_, agent, _) = run(yaml, []).await;
        let task = &agent.calls[0];
        assert_eq!(task.become_user, "inherited-user");
        assert_eq!(task.environment["INHERITED"], "inherited-value");
        assert_eq!(task.environment["OVERRIDE"], "task-value");
        assert_eq!(task.iterations[0].free_form, "echo inherited-value");
    }

    #[tokio::test]
    async fn block_always_runs_on_success() {
        let yaml = "- hosts: all\n  tasks:\n    - name: guarded\n      block:\n        - name: body\n          command: echo body\n      always:\n        - name: cleanup\n          command: echo cleanup\n";
        let (_, agent, _) = run(yaml, []).await;
        assert_eq!(agent.calls.len(), 2);
        assert_eq!(agent.calls[1].name, "cleanup");
    }

    #[tokio::test]
    async fn block_always_runs_after_rescue() {
        let yaml = "- hosts: all\n  tasks:\n    - name: guarded\n      block:\n        - name: body\n          command: echo body\n      rescue:\n        - name: rescue\n          command: echo rescue\n      always:\n        - name: cleanup\n          command: echo cleanup\n";
        let results = [
            serde_json::json!({"changed": false, "failed": true}),
            serde_json::json!({"changed": false, "failed": false}),
            serde_json::json!({"changed": false, "failed": false}),
        ];
        let (recap, agent, _) = run(yaml, results).await;
        assert_eq!(recap.rescued, 1);
        assert_eq!(agent.calls.len(), 3);
        assert_eq!(agent.calls[2].name, "cleanup");
    }

    #[tokio::test]
    async fn block_always_runs_before_unrescued_failure_stops_host() {
        let yaml = "- hosts: all\n  tasks:\n    - name: guarded\n      block:\n        - name: body\n          command: echo body\n      always:\n        - name: cleanup\n          command: echo cleanup\n    - name: must not run\n      command: echo after\n";
        let results = [
            serde_json::json!({"changed": false, "failed": true}),
            serde_json::json!({"changed": false, "failed": false}),
        ];
        let (_, agent, _) = run(yaml, results).await;
        assert_eq!(agent.calls.len(), 2);
        assert_eq!(agent.calls[1].name, "cleanup");
    }

    #[tokio::test]
    async fn apply_revalidates_templated_enum_values() {
        let yaml = "- hosts: all\n  tasks:\n    - name: invalid rendered enum\n      filesystem:\n        dev: /dev/synthetic\n        fstype: '{{ ansible_architecture }}'\n";
        let playbook = ruxel_core::playbook::parse("test.yml", yaml).unwrap();
        let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
        let compiled = ruxel_core::compiler::compile(&playbook, &engine).unwrap();
        let mut agent = FakeAgent::default();
        let mut output = Vec::new();
        let mut event_log = Vec::new();
        let error = run_play(
            &playbook.plays[0],
            &compiled.plays[0],
            "test-host",
            &v1::Facts {
                architecture: "btrfs".into(),
                ..Default::default()
            },
            &engine,
            &mut agent,
            std::path::Path::new("."),
            OutputFormat::Human,
            None,
            &mut output,
            &mut event_log,
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("outside the observed value set"),
            "{error:#}"
        );
        assert!(agent.calls.is_empty());
    }
}

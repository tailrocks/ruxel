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
use ruxel_core::engine::{Engine, Scope, VarValue};
use ruxel_core::playbook::{Condition, Play, Task, TaskBody};
use ruxel_core::task_eval;
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
use std::collections::BTreeSet;
use std::io::Write;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recap {
    pub ok: u32,
    pub changed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub rescued: u32,
    pub ignored: u32,
}

struct HostRun<'a> {
    engine: &'a Engine,
    conn: &'a mut AgentConnection,
    host: String,
    play_vars: Vec<(String, VarValue)>,
    facts: Vec<(String, VarValue)>,
    registered: Vec<(String, VarValue)>,
    notified: BTreeSet<String>,
    recap: Recap,
    next_task_id: u64,
}

pub async fn run_play(
    play: &Play,
    host: &str,
    facts: &v1::Facts,
    engine: &Engine,
    conn: &mut AgentConnection,
    out: &mut impl Write,
) -> Result<Recap> {
    let mut run = HostRun {
        engine,
        conn,
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
    };

    let mut host_failed = false;
    'sections: for section in [&play.pre_tasks, &play.tasks] {
        for task in section.iter() {
            if run.run_task_or_block(task, out).await? {
                host_failed = true;
                break 'sections;
            }
        }
    }

    // Handlers flush at end of play, definition order, once each, only if
    // notified by a changed task (SEMANTICS §4).
    if !host_failed {
        for handler in &play.handlers {
            let name = handler.name.clone().unwrap_or_default();
            if run.notified.contains(&name) && run.run_task_or_block(handler, out).await? {
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

impl HostRun<'_> {
    fn scope(&self, task_vars: &[(String, serde_norway::Value)]) -> Scope {
        let mut scope = Scope::new()
            .with_layer(self.play_vars.clone())
            .with_layer(self.facts.clone())
            .with_layer(self.registered.clone());
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

    /// Returns true when the host must stop (unrescued failure).
    async fn run_task_or_block(&mut self, task: &Task, out: &mut impl Write) -> Result<bool> {
        if let TaskBody::Block {
            block,
            rescue,
            always: _,
        } = &task.body
        {
            // Block-level when gates the whole block (SEMANTICS §4).
            if let Some(when) = &task.when {
                let scope = self.scope(&task.vars);
                if !self.engine.eval_condition(when, &scope)? {
                    for sub in block {
                        self.print_status(out, sub, "skipped", None);
                        self.recap.skipped += 1;
                    }
                    return Ok(false);
                }
            }
            let mut block_failed = false;
            for sub in block {
                if Box::pin(self.run_task_or_block(sub, out)).await? {
                    block_failed = true;
                    break;
                }
            }
            if block_failed {
                if rescue.is_empty() {
                    return Ok(true);
                }
                self.recap.rescued += 1;
                for sub in rescue {
                    if Box::pin(self.run_task_or_block(sub, out)).await? {
                        return Ok(true); // rescue itself failed
                    }
                }
            }
            return Ok(false);
        }

        self.run_module_task(task, out).await
    }

    async fn run_module_task(&mut self, task: &Task, out: &mut impl Write) -> Result<bool> {
        let TaskBody::Module(call) = &task.body else {
            unreachable!("blocks handled by caller")
        };
        let scope = self.scope(&task.vars);

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
                        self.finish_task(task, out, skip, "skipped", false);
                        return Ok(false);
                    }
                }
                self.execute_iterations(task, call, vec![(None, scope.clone())], out)
                    .await?
            }
            Some(items) => {
                if items.is_empty() {
                    let agg = task_eval::loop_aggregate(vec![]);
                    self.finish_task(task, out, agg, "skipped", false);
                    return Ok(false);
                }
                let mut iterations: Vec<(Option<Value>, Scope)> = Vec::new();
                for item in items {
                    let item_scope =
                        scope.with_layer(vec![("item".to_string(), VarValue::Final(item.clone()))]);
                    iterations.push((Some(item), item_scope));
                }
                self.execute_iterations(task, call, iterations, out).await?
            }
        };

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

        Ok(failed && !task.ignore_errors)
    }

    /// Execute the task's iterations (single or per-item), including
    /// controller-side modules, until/retries, and per-item when handled
    /// by the caller for loops.
    async fn execute_iterations(
        &mut self,
        task: &Task,
        call: &ruxel_core::playbook::ModuleCall,
        iterations: Vec<(Option<Value>, Scope)>,
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
                    .execute_once(task, call, &item_scope, item.as_ref())
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
        scope: &Scope,
        item: Option<&Value>,
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
                bail!("pause relay is not wired yet (M3 in progress)");
            }
            _ => {}
        }

        // Agent-side execution: render params + free-form with the item
        // scope, ship one iteration, await its result.
        let mut params = serde_json::Map::new();
        for (k, v) in &call.params {
            let rendered = self.engine.render_value(v, scope)?;
            params.insert(k.clone(), serde_json::to_value(&rendered)?);
        }
        let free_form = match &call.free_form {
            Some(body) => self
                .engine
                .render_str(body, scope)?
                .as_str()
                .map(str::to_string)
                .unwrap_or_default(),
            None => String::new(),
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

        self.conn
            .send(&v1::Envelope {
                msg: Some(EnvMsg::Plan(v1::Plan {
                    tasks: vec![v1::RenderedTask {
                        task_id,
                        name: label(task),
                        module: module.to_string(),
                        rendered: true,
                        iterations: vec![v1::Iteration {
                            item_label,
                            params_json: serde_json::to_vec(&params)?,
                            free_form,
                        }],
                        check_mode_override: task.check_mode == Some(false),
                        no_log: task.no_log,
                        become_user: task.become_user.clone().unwrap_or_default(),
                        environment,
                    }],
                    blobs_referenced: vec![],
                })),
            })
            .await?;

        loop {
            let event = self
                .conn
                .next_event()
                .await?
                .context("agent closed mid-task")?;
            match event.msg {
                Some(EvMsg::TaskStart(_)) => continue,
                Some(EvMsg::Log(_)) => continue,
                Some(EvMsg::TaskResult(res)) if res.task_id == task_id => {
                    let json: serde_json::Value = if res.result_json.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_slice(&res.result_json)?
                    };
                    return Ok(to_mj(json));
                }
                Some(EvMsg::Crash(c)) => {
                    bail!("agent crashed: {} at {}", c.message, c.location)
                }
                other => bail!("unexpected agent event mid-task: {other:?}"),
            }
        }
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
            String::new()
        } else if status == "failed" {
            serde_json::to_string(&result).unwrap_or_default()
        } else {
            String::new()
        };
        self.print_status(out, task, status, None);
        if !display.is_empty() {
            let _ = writeln!(out, "    {display}");
        }
    }

    fn print_status(&self, out: &mut impl Write, task: &Task, status: &str, item: Option<&str>) {
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
}

fn label(task: &Task) -> String {
    task.name.clone().unwrap_or_else(|| "(unnamed)".into())
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

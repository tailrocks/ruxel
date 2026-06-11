//! The plan compiler (ARCHITECTURE §3.3, §4): annotate every task with the
//! registered/fact names its templates read, expand and render everything
//! whose inputs are already known (play vars + memoized lookups), and mark
//! the rest as deferred nodes to be rendered when their register inputs
//! arrive at run time. Rendered literal-enum params are re-validated
//! against the closed surface — a templated `state:` may not escape it.

use crate::engine::{Engine, EngineError, Scope, VarValue};
use crate::playbook::{Condition, ErrorKind, Play, Playbook, Task, TaskBody};
use minijinja::value::Value;
use std::collections::BTreeSet;

/// Facts the agent supplies at handshake (SEMANTICS §2) — runtime inputs,
/// like registers, from the compiler's point of view.
const FACT_NAMES: &[&str] = &[
    "ansible_default_ipv4",
    "ansible_facts",
    "ansible_architecture",
];

#[derive(Debug)]
pub struct Plan {
    pub source: String,
    pub plays: Vec<PlayPlan>,
}

#[derive(Debug)]
pub struct PlayPlan {
    pub name: Option<String>,
    pub hosts: String,
    pub pre_tasks: Vec<PlanTask>,
    pub tasks: Vec<PlanTask>,
    pub handlers: Vec<PlanTask>,
}

#[derive(Debug)]
pub struct PlanTask {
    pub name: Option<String>,
    pub body: PlanBody,
    /// Registered/fact names read anywhere in this task's templates.
    pub reads: BTreeSet<String>,
    /// Names this task provides to later tasks (register, set_fact keys).
    pub provides: Vec<String>,
    pub no_log: bool,
}

#[derive(Debug)]
pub enum PlanBody {
    Module {
        module: &'static str,
        readiness: Readiness,
    },
    Block {
        block: Vec<PlanTask>,
        rescue: Vec<PlanTask>,
        always: Vec<PlanTask>,
    },
}

#[derive(Debug)]
pub enum Readiness {
    /// All inputs known at compile time: params (and loop items, when a
    /// loop is present) are fully rendered.
    Static {
        params: Vec<(String, Value)>,
        free_form: Option<String>,
        /// One rendered param set per loop item (loop tasks only).
        loop_items: Option<Vec<Value>>,
    },
    /// Reads names that only exist at run time; rendered when they arrive.
    Deferred { waits_on: BTreeSet<String> },
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("{playbook}: task {task:?}: render: {source}")]
    Render {
        playbook: String,
        task: String,
        source: EngineError,
    },
    #[error("{playbook}: task {task:?}: {kind}")]
    Surface {
        playbook: String,
        task: String,
        kind: ErrorKind,
    },
}

pub fn compile(playbook: &Playbook, engine: &Engine) -> Result<Plan, Box<CompileError>> {
    let mut plays = Vec::new();
    for play in &playbook.plays {
        plays.push(compile_play(playbook, play, engine)?);
    }
    Ok(Plan {
        source: playbook.source.clone(),
        plays,
    })
}

fn compile_play(
    playbook: &Playbook,
    play: &Play,
    engine: &Engine,
) -> Result<PlayPlan, Box<CompileError>> {
    // The provider universe: every register/set_fact name any task in the
    // play introduces, plus the handshake facts.
    let mut providers: BTreeSet<String> = FACT_NAMES.iter().map(|s| s.to_string()).collect();
    for task in play
        .pre_tasks
        .iter()
        .chain(&play.tasks)
        .chain(&play.handlers)
    {
        collect_providers(task, &mut providers);
    }

    let scope = play_scope(play);
    let mut ctx = PlayCtx {
        playbook: &playbook.source,
        engine,
        providers: &providers,
        scope,
    };

    Ok(PlayPlan {
        name: play.name.clone(),
        hosts: play.hosts.clone(),
        pre_tasks: compile_tasks(&play.pre_tasks, &mut ctx)?,
        tasks: compile_tasks(&play.tasks, &mut ctx)?,
        handlers: compile_tasks(&play.handlers, &mut ctx)?,
    })
}

fn play_scope(play: &Play) -> Scope {
    Scope::new().with_layer(
        play.vars
            .iter()
            .map(|(k, v)| (k.clone(), VarValue::Raw(v.clone())))
            .collect(),
    )
}

fn collect_providers(task: &Task, out: &mut BTreeSet<String>) {
    if let Some(reg) = &task.register {
        out.insert(reg.clone());
    }
    if let TaskBody::Module(call) = &task.body
        && call.module.name == "set_fact"
    {
        for (k, _) in &call.params {
            out.insert(k.clone());
        }
    }
    if let TaskBody::Block {
        block,
        rescue,
        always,
    } = &task.body
    {
        for t in block.iter().chain(rescue).chain(always) {
            collect_providers(t, out);
        }
    }
}

struct PlayCtx<'a> {
    playbook: &'a str,
    engine: &'a Engine,
    providers: &'a BTreeSet<String>,
    scope: Scope,
}

fn compile_tasks(
    tasks: &[Task],
    ctx: &mut PlayCtx<'_>,
) -> Result<Vec<PlanTask>, Box<CompileError>> {
    tasks.iter().map(|t| compile_task(t, ctx)).collect()
}

fn compile_task(task: &Task, ctx: &mut PlayCtx<'_>) -> Result<PlanTask, Box<CompileError>> {
    let task_label = task.name.clone().unwrap_or_else(|| "(unnamed)".into());
    let mut reads = BTreeSet::new();
    scan_task_reads(task, ctx.providers, &mut reads);

    let mut provides = Vec::new();
    if let Some(reg) = &task.register {
        provides.push(reg.clone());
    }

    let body = match &task.body {
        TaskBody::Block {
            block,
            rescue,
            always,
        } => PlanBody::Block {
            block: compile_tasks(block, ctx)?,
            rescue: compile_tasks(rescue, ctx)?,
            always: compile_tasks(always, ctx)?,
        },
        TaskBody::Module(call) => {
            if call.module.name == "set_fact" {
                for (k, _) in &call.params {
                    provides.push(k.clone());
                }
            }
            if reads.is_empty() {
                let rendered = render_static(task, ctx).map_err(|source| {
                    Box::new(CompileError::Render {
                        playbook: ctx.playbook.to_string(),
                        task: task_label.clone(),
                        source,
                    })
                })?;
                validate_rendered_enums(call.module, &rendered.0, ctx.playbook, &task_label)?;
                PlanBody::Module {
                    module: call.module.name,
                    readiness: Readiness::Static {
                        params: rendered.0,
                        free_form: rendered.1,
                        loop_items: rendered.2,
                    },
                }
            } else {
                PlanBody::Module {
                    module: call.module.name,
                    readiness: Readiness::Deferred {
                        waits_on: reads.clone(),
                    },
                }
            }
        }
    };

    Ok(PlanTask {
        name: task.name.clone(),
        body,
        reads,
        provides,
        no_log: task.no_log,
    })
}

type RenderedParts = (Vec<(String, Value)>, Option<String>, Option<Vec<Value>>);

/// Render a fully-static task: loop items first (native list), then params
/// once (loop-less) — per-item param rendering happens at dispatch, where
/// `item` is bound, but compile proves them renderable by rendering with
/// each item now.
fn render_static(task: &Task, ctx: &PlayCtx<'_>) -> Result<RenderedParts, EngineError> {
    let TaskBody::Module(call) = &task.body else {
        unreachable!("blocks handled by caller")
    };
    let scope = if task.vars.is_empty() {
        ctx.scope.clone()
    } else {
        ctx.scope.with_layer(
            task.vars
                .iter()
                .map(|(k, v)| (k.clone(), VarValue::Raw(v.clone())))
                .collect(),
        )
    };

    let loop_items: Option<Vec<Value>> = match &task.loop_ {
        None => None,
        Some(value) => {
            let rendered = ctx.engine.render_value(value, &scope)?;
            let items: Vec<Value> = rendered.try_iter()?.collect();
            Some(items)
        }
    };

    let render_params_with = |scope: &Scope| -> Result<RenderedParts, EngineError> {
        let mut params = Vec::new();
        for (k, v) in &call.params {
            params.push((k.clone(), ctx.engine.render_value(v, scope)?));
        }
        let free_form = match &call.free_form {
            Some(body) => Some(
                ctx.engine
                    .render_str(body, scope)?
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| body.clone()),
            ),
            None => None,
        };
        Ok((params, free_form, None))
    };

    match &loop_items {
        None => render_params_with(&scope),
        Some(items) => {
            // Params are proven renderable per item; the first item's
            // rendering is kept as the representative param shape.
            let mut first: Option<RenderedParts> = None;
            for item in items {
                let item_scope =
                    scope.with_layer(vec![("item".to_string(), VarValue::Final(item.clone()))]);
                let rendered = render_params_with(&item_scope)?;
                if first.is_none() {
                    first = Some(rendered);
                }
            }
            let (params, free_form, _) = first.unwrap_or_else(|| (Vec::new(), None, None));
            Ok((params, free_form, loop_items))
        }
    }
}

/// Templated literal-enum params are validated post-render (the parser let
/// them through on the promise we would).
fn validate_rendered_enums(
    module: &crate::modules::ModuleSurface,
    params: &[(String, Value)],
    playbook: &str,
    task: &str,
) -> Result<(), Box<CompileError>> {
    for (param, values) in module.literal_enums {
        if let Some((_, v)) = params.iter().find(|(k, _)| k == param)
            && let Some(s) = v.as_str()
            && !values.contains(&s)
        {
            return Err(Box::new(CompileError::Surface {
                playbook: playbook.to_string(),
                task: task.to_string(),
                kind: ErrorKind::ValueOutsideSurface {
                    module: module.name.into(),
                    param: param.to_string(),
                    value: s.to_string(),
                    allowed: values.iter().map(|v| v.to_string()).collect(),
                },
            }));
        }
    }
    Ok(())
}

// -- Read-set scanning --------------------------------------------------------

fn scan_task_reads(task: &Task, providers: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    let mut scan_cond = |c: &Option<Condition>| {
        if let Some(c) = c {
            match c {
                Condition::Literal(_) => {}
                Condition::Expr(e) => scan_expr(e, providers, out),
                Condition::All(es) => es.iter().for_each(|e| scan_expr(e, providers, out)),
            }
        }
    };
    scan_cond(&task.when);
    scan_cond(&task.until);
    scan_cond(&task.changed_when);
    scan_cond(&task.failed_when);

    if let Some(loop_value) = &task.loop_ {
        scan_yaml(loop_value, providers, out);
    }
    for (_, v) in &task.vars {
        scan_yaml(v, providers, out);
    }
    for (_, v) in &task.environment {
        scan_yaml(v, providers, out);
    }
    if let TaskBody::Module(call) = &task.body {
        for (_, v) in &call.params {
            scan_yaml(v, providers, out);
        }
        if let Some(body) = &call.free_form {
            scan_template(body, providers, out);
        }
    }
}

fn scan_yaml(
    value: &serde_norway::Value,
    providers: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    use serde_norway::Value as Yaml;
    match value {
        Yaml::String(s) => scan_template(s, providers, out),
        Yaml::Sequence(items) => items.iter().for_each(|v| scan_yaml(v, providers, out)),
        Yaml::Mapping(map) => map.iter().for_each(|(_, v)| scan_yaml(v, providers, out)),
        _ => {}
    }
}

/// Scan only the `{{ … }}` / `{% … %}` segments of a template string.
fn scan_template(template: &str, providers: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    let bytes = template.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let (open, close) = match &bytes[i..i + 2] {
            b"{{" => ("{{", "}}"),
            b"{%" => ("{%", "%}"),
            _ => {
                i += 1;
                continue;
            }
        };
        let start = i + open.len();
        let Some(end_rel) = template[start..].find(close) else {
            break;
        };
        scan_expr(&template[start..start + end_rel], providers, out);
        i = start + end_rel + close.len();
    }
}

/// Tokenize identifiers (string literals skipped) and keep the ones that
/// name a runtime provider. Attribute segments after `.` can't collide:
/// only exact provider names are kept.
fn scan_expr(expr: &str, providers: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    let mut chars = expr.char_indices().peekable();
    let mut quote: Option<char> = None;
    let mut prev_was_dot = false;
    while let Some((_, c)) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut ident = String::new();
                ident.push(c);
                while let Some((_, n)) = chars.peek() {
                    if n.is_ascii_alphanumeric() || *n == '_' {
                        ident.push(*n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if !prev_was_dot && providers.contains(&ident) {
                    out.insert(ident);
                }
                prev_was_dot = false;
            }
            '.' => prev_was_dot = true,
            c if c.is_whitespace() => {}
            _ => prev_was_dot = false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{DrySecrets, MemoizedResolver};
    use std::sync::Arc;

    fn compile_yaml(yaml: &str) -> Plan {
        let pb = crate::playbook::parse("test.yml", yaml).unwrap();
        let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
        compile(&pb, &engine).unwrap()
    }

    #[test]
    fn static_task_renders_params() {
        let plan = compile_yaml(
            r#"
- hosts: all
  vars:
    base: /opt/demo
  tasks:
    - name: dir
      file:
        path: "{{ base }}/data"
        state: directory
"#,
        );
        let PlanBody::Module { readiness, .. } = &plan.plays[0].tasks[0].body else {
            panic!()
        };
        let Readiness::Static { params, .. } = readiness else {
            panic!("expected static")
        };
        assert_eq!(params[0].1.as_str(), Some("/opt/demo/data"));
    }

    #[test]
    fn register_consumer_is_deferred_and_annotated() {
        let plan = compile_yaml(
            r#"
- hosts: all
  tasks:
    - name: probe
      stat:
        path: /etc/thing
      register: thing_stat
    - name: use
      file:
        path: /etc/thing
        state: absent
      when: thing_stat.stat.exists
"#,
        );
        let t = &plan.plays[0].tasks[1];
        assert!(t.reads.contains("thing_stat"));
        let PlanBody::Module { readiness, .. } = &t.body else {
            panic!()
        };
        assert!(
            matches!(readiness, Readiness::Deferred { waits_on } if waits_on.contains("thing_stat"))
        );
        assert_eq!(plan.plays[0].tasks[0].provides, vec!["thing_stat"]);
    }

    #[test]
    fn fact_reads_defer() {
        let plan = compile_yaml(
            "- hosts: all\n  tasks:\n    - debug:\n        msg: \"{{ ansible_architecture }}\"\n",
        );
        let t = &plan.plays[0].tasks[0];
        assert!(t.reads.contains("ansible_architecture"));
    }

    #[test]
    fn static_loop_expands_items() {
        let plan = compile_yaml(
            r#"
- hosts: all
  vars:
    dirs: [a, b, c]
  tasks:
    - file:
        path: "/opt/{{ item }}"
        state: directory
      loop: "{{ dirs }}"
"#,
        );
        let PlanBody::Module { readiness, .. } = &plan.plays[0].tasks[0].body else {
            panic!()
        };
        let Readiness::Static { loop_items, .. } = readiness else {
            panic!("expected static")
        };
        assert_eq!(loop_items.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn rendered_enum_outside_surface_fails_compile() {
        let pb = crate::playbook::parse(
            "test.yml",
            r#"
- hosts: all
  vars:
    desired: touch
  tasks:
    - file:
        path: /x
        state: "{{ desired }}"
"#,
        )
        .unwrap();
        let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
        let err = compile(&pb, &engine).unwrap_err();
        assert!(matches!(*err, CompileError::Surface { .. }));
    }

    #[test]
    fn quoted_provider_names_are_not_reads() {
        let plan = compile_yaml(
            r#"
- hosts: all
  tasks:
    - command: echo x
      register: docker_list
    - debug:
        msg: "the string 'docker_list' is quoted {{ 'docker_list' }}"
"#,
        );
        assert!(plan.plays[0].tasks[1].reads.is_empty());
    }

    #[test]
    fn set_fact_provides_its_keys() {
        let plan = compile_yaml(
            r#"
- hosts: all
  tasks:
    - set_fact:
        my_flag: "yes"
    - debug:
        msg: "{{ my_flag }}"
"#,
        );
        assert_eq!(plan.plays[0].tasks[0].provides, vec!["my_flag"]);
        assert!(plan.plays[0].tasks[1].reads.contains("my_flag"));
    }
}

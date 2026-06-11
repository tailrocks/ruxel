//! Playbook YAML → typed model, enforcing the closed surface
//! (docs/SEMANTICS.md §§1–4, §6). Template strings stay raw at this stage —
//! rendering is the engine's job, not the parser's.

use crate::modules::{self, ModuleSurface};
use serde_norway::Value;

#[derive(Debug)]
pub struct Playbook {
    pub source: String,
    pub plays: Vec<Play>,
}

#[derive(Debug)]
pub struct Play {
    pub name: Option<String>,
    pub hosts: String,
    pub becomes: bool,
    /// Play vars in file order, values raw (lazily templated).
    pub vars: Vec<(String, Value)>,
    pub pre_tasks: Vec<Task>,
    pub tasks: Vec<Task>,
    pub handlers: Vec<Task>,
}

#[derive(Debug)]
pub struct Task {
    pub name: Option<String>,
    pub body: TaskBody,
    pub when: Option<Condition>,
    pub register: Option<String>,
    /// Raw `loop:` value — literal list or `"{{ var }}"` template string.
    pub loop_: Option<Value>,
    pub loop_label: Option<String>,
    pub vars: Vec<(String, Value)>,
    pub tags: Vec<String>,
    pub notify: Vec<String>,
    pub becomes: Option<bool>,
    pub become_user: Option<String>,
    pub changed_when: Option<Condition>,
    pub failed_when: Option<Condition>,
    pub ignore_errors: bool,
    /// `check_mode: no` forces real execution under `--check` (SEMANTICS §3.5).
    pub check_mode: Option<bool>,
    pub no_log: bool,
    pub environment: Vec<(String, Value)>,
    pub until: Option<Condition>,
    pub retries: Option<u64>,
    pub delay: Option<u64>,
}

#[derive(Debug)]
pub enum TaskBody {
    Module(ModuleCall),
    Block {
        block: Vec<Task>,
        rescue: Vec<Task>,
        always: Vec<Task>,
    },
}

#[derive(Debug)]
pub struct ModuleCall {
    pub module: &'static ModuleSurface,
    /// Param key/value pairs in file order, raw values.
    pub params: Vec<(String, Value)>,
    /// Free-form body for `command`/`shell`.
    pub free_form: Option<String>,
}

/// `when`/`changed_when`/`failed_when`/`until`: a bare expression, a literal
/// bool, or a list of expressions joined by AND (SEMANTICS §3.2).
#[derive(Debug, Clone)]
pub enum Condition {
    Literal(bool),
    Expr(String),
    All(Vec<String>),
}

#[derive(Debug, thiserror::Error)]
#[error("{source_name}: {at}: {kind}")]
pub struct ParseError {
    pub source_name: String,
    /// Human-readable location: play/task names.
    pub at: String,
    pub kind: ErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ErrorKind {
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("expected {expected}, got {got}")]
    Shape { expected: &'static str, got: String },
    #[error(
        "unknown play keyword {0:?} (closed surface: name, hosts, become, vars, pre_tasks, tasks, handlers)"
    )]
    UnknownPlayKey(String),
    #[error("unknown task keyword or module {0:?} — not in the closed surface (docs/SEMANTICS.md)")]
    UnknownTaskKey(String),
    #[error("task has no module and no block")]
    NoModule,
    #[error("task mixes two modules or a module with a block: {0:?} and {1:?}")]
    TwoModules(String, String),
    #[error("module {module}: unknown parameter {param:?} (closed surface)")]
    UnknownParam { module: String, param: String },
    #[error(
        "module {module}: parameter {param}={value:?} outside the observed value set {allowed:?}"
    )]
    ValueOutsideSurface {
        module: String,
        param: String,
        value: String,
        allowed: Vec<String>,
    },
    #[error("module {module} does not take a free-form string body")]
    UnexpectedFreeForm { module: String },
    #[error("`args:` is only used with shell in the workload, found on {module}")]
    ArgsNotAllowed { module: String },
    #[error("expected boolean (true/false/yes/no), got {0:?}")]
    NotABool(String),
}

type Result<T> = std::result::Result<T, ErrorKind>;

/// Parse one playbook file's content.
pub fn parse(source_name: &str, content: &str) -> std::result::Result<Playbook, ParseError> {
    let err = |at: String, kind: ErrorKind| ParseError {
        source_name: source_name.to_string(),
        at,
        kind,
    };
    let doc: Value = serde_norway::from_str(content).map_err(|e| err("(file)".into(), e.into()))?;
    let Value::Sequence(plays_raw) = doc else {
        return Err(err(
            "(file)".into(),
            ErrorKind::Shape {
                expected: "a list of plays",
                got: type_name(&doc).into(),
            },
        ));
    };
    let mut plays = Vec::new();
    for (i, play_raw) in plays_raw.into_iter().enumerate() {
        let at = format!("play[{i}]");
        plays.push(parse_play(play_raw).map_err(|k| err(at, k))?);
    }
    Ok(Playbook {
        source: source_name.to_string(),
        plays,
    })
}

fn parse_play(value: Value) -> Result<Play> {
    let map = as_mapping(value, "a play mapping")?;
    let mut play = Play {
        name: None,
        hosts: String::new(),
        becomes: false,
        vars: Vec::new(),
        pre_tasks: Vec::new(),
        tasks: Vec::new(),
        handlers: Vec::new(),
    };
    for (k, v) in map {
        let key = as_string_key(&k)?;
        match key.as_str() {
            "name" => play.name = Some(string_value(v)?),
            "hosts" => play.hosts = string_value(v)?,
            "become" => play.becomes = bool_value(&v)?,
            "vars" => play.vars = ordered_pairs(v)?,
            "pre_tasks" => play.pre_tasks = parse_tasks(v)?,
            "tasks" => play.tasks = parse_tasks(v)?,
            "handlers" => play.handlers = parse_tasks(v)?,
            _ => return Err(ErrorKind::UnknownPlayKey(key)),
        }
    }
    Ok(play)
}

fn parse_tasks(value: Value) -> Result<Vec<Task>> {
    let Value::Sequence(items) = value else {
        return Err(ErrorKind::Shape {
            expected: "a list of tasks",
            got: type_name(&value).into(),
        });
    };
    items.into_iter().map(parse_task).collect()
}

fn parse_task(value: Value) -> Result<Task> {
    let map = as_mapping(value, "a task mapping")?;
    let mut task = Task {
        name: None,
        body: TaskBody::Module(ModuleCall {
            module: modules::lookup("debug").expect("debug registered"),
            params: Vec::new(),
            free_form: None,
        }),
        when: None,
        register: None,
        loop_: None,
        loop_label: None,
        vars: Vec::new(),
        tags: Vec::new(),
        notify: Vec::new(),
        becomes: None,
        become_user: None,
        changed_when: None,
        failed_when: None,
        ignore_errors: false,
        check_mode: None,
        no_log: false,
        environment: Vec::new(),
        until: None,
        retries: None,
        delay: None,
    };

    let mut module: Option<(String, Value)> = None;
    let mut args: Option<Value> = None;
    let mut block_parts: Option<(Vec<Task>, Vec<Task>, Vec<Task>)> = None;

    for (k, v) in map {
        let key = as_string_key(&k)?;
        match key.as_str() {
            "name" => task.name = Some(string_value(v)?),
            "when" => task.when = Some(parse_condition(v)?),
            "register" => task.register = Some(string_value(v)?),
            "loop" => task.loop_ = Some(v),
            "loop_control" => {
                for (ck, cv) in ordered_pairs(v)? {
                    match ck.as_str() {
                        "label" => task.loop_label = Some(string_value(cv)?),
                        other => {
                            return Err(ErrorKind::UnknownTaskKey(format!("loop_control.{other}")));
                        }
                    }
                }
            }
            "vars" => task.vars = ordered_pairs(v)?,
            "tags" => task.tags = string_list(v)?,
            "notify" => task.notify = string_list(v)?,
            "become" => task.becomes = Some(bool_value(&v)?),
            "become_user" => task.become_user = Some(string_value(v)?),
            "changed_when" => task.changed_when = Some(parse_condition(v)?),
            "failed_when" => task.failed_when = Some(parse_condition(v)?),
            "ignore_errors" => task.ignore_errors = bool_value(&v)?,
            "check_mode" => task.check_mode = Some(bool_value(&v)?),
            "no_log" => task.no_log = bool_value(&v)?,
            "environment" => task.environment = ordered_pairs(v)?,
            "until" => task.until = Some(parse_condition(v)?),
            "retries" => task.retries = Some(u64_value(&v)?),
            "delay" => task.delay = Some(u64_value(&v)?),
            "args" => args = Some(v),
            "block" => {
                let parts = block_parts.get_or_insert((vec![], vec![], vec![]));
                parts.0 = parse_tasks(v)?;
            }
            "rescue" => {
                let parts = block_parts.get_or_insert((vec![], vec![], vec![]));
                parts.1 = parse_tasks(v)?;
            }
            "always" => {
                let parts = block_parts.get_or_insert((vec![], vec![], vec![]));
                parts.2 = parse_tasks(v)?;
            }
            _ => {
                if modules::lookup(&key).is_some() {
                    if let Some((prev, _)) = &module {
                        return Err(ErrorKind::TwoModules(prev.clone(), key));
                    }
                    module = Some((key, v));
                } else {
                    return Err(ErrorKind::UnknownTaskKey(key));
                }
            }
        }
    }

    task.body = match (module, block_parts) {
        (Some((name, _)), Some(_)) => return Err(ErrorKind::TwoModules(name, "block".into())),
        (None, Some((block, rescue, always))) => {
            if let Some(a) = args {
                let _ = a;
                return Err(ErrorKind::ArgsNotAllowed {
                    module: "block".into(),
                });
            }
            TaskBody::Block {
                block,
                rescue,
                always,
            }
        }
        (Some((name, params)), None) => TaskBody::Module(parse_module_call(&name, params, args)?),
        (None, None) => return Err(ErrorKind::NoModule),
    };
    Ok(task)
}

fn parse_module_call(name: &str, value: Value, args: Option<Value>) -> Result<ModuleCall> {
    let surface = modules::lookup(name).expect("checked by caller");
    let mut call = ModuleCall {
        module: surface,
        params: Vec::new(),
        free_form: None,
    };

    match value {
        Value::String(body) => {
            if !surface.free_form {
                return Err(ErrorKind::UnexpectedFreeForm {
                    module: name.into(),
                });
            }
            call.free_form = Some(body);
        }
        Value::Mapping(_) => {
            for (param, pv) in ordered_pairs(value)? {
                validate_param(surface, &param, &pv, surface.params)?;
                call.params.push((param, pv));
            }
        }
        Value::Null => {}
        other => {
            return Err(ErrorKind::Shape {
                expected: "module params mapping or free-form string",
                got: type_name(&other).into(),
            });
        }
    }

    if let Some(args_value) = args {
        if surface.args_params.is_empty() {
            return Err(ErrorKind::ArgsNotAllowed {
                module: name.into(),
            });
        }
        for (param, pv) in ordered_pairs(args_value)? {
            validate_param(surface, &param, &pv, surface.args_params)?;
            call.params.push((param, pv));
        }
    }
    Ok(call)
}

fn validate_param(
    surface: &ModuleSurface,
    param: &str,
    value: &Value,
    allowed: &[&str],
) -> Result<()> {
    if !surface.any_params && !allowed.contains(&param) {
        return Err(ErrorKind::UnknownParam {
            module: surface.name.into(),
            param: param.into(),
        });
    }
    if let Some((_, values)) = surface.literal_enums.iter().find(|(p, _)| *p == param) {
        if let Value::String(s) = value {
            // Templated values are validated post-render, not here.
            if !s.contains("{{") && !values.contains(&s.as_str()) {
                return Err(ErrorKind::ValueOutsideSurface {
                    module: surface.name.into(),
                    param: param.into(),
                    value: s.clone(),
                    allowed: values.iter().map(|v| v.to_string()).collect(),
                });
            }
        }
    }
    Ok(())
}

fn parse_condition(value: Value) -> Result<Condition> {
    Ok(match value {
        Value::Bool(b) => Condition::Literal(b),
        Value::String(s) => Condition::Expr(s),
        Value::Sequence(items) => Condition::All(
            items
                .into_iter()
                .map(string_value)
                .collect::<Result<Vec<_>>>()?,
        ),
        other => {
            return Err(ErrorKind::Shape {
                expected: "a condition expression, bool, or list of expressions",
                got: type_name(&other).into(),
            });
        }
    })
}

// -- YAML helpers -----------------------------------------------------------

fn as_mapping(value: Value, expected: &'static str) -> Result<serde_norway::Mapping> {
    match value {
        Value::Mapping(m) => Ok(m),
        other => Err(ErrorKind::Shape {
            expected,
            got: type_name(&other).into(),
        }),
    }
}

fn ordered_pairs(value: Value) -> Result<Vec<(String, Value)>> {
    as_mapping(value, "a mapping")?
        .into_iter()
        .map(|(k, v)| Ok((as_string_key(&k)?, v)))
        .collect()
}

fn as_string_key(key: &Value) -> Result<String> {
    match key {
        Value::String(s) => Ok(s.clone()),
        other => Err(ErrorKind::Shape {
            expected: "a string key",
            got: type_name(other).into(),
        }),
    }
}

fn string_value(value: Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(ErrorKind::Shape {
            expected: "a string",
            got: type_name(&other).into(),
        }),
    }
}

fn string_list(value: Value) -> Result<Vec<String>> {
    match value {
        Value::String(s) => Ok(vec![s]),
        Value::Sequence(items) => items.into_iter().map(string_value).collect(),
        other => Err(ErrorKind::Shape {
            expected: "a string or list of strings",
            got: type_name(&other).into(),
        }),
    }
}

/// YAML-1.1-style truthiness, as Ansible's loader applies it: real bools plus
/// the yes/no/true/false string spellings the workload uses.
fn bool_value(value: &Value) -> Result<bool> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => match s.as_str() {
            "yes" | "true" | "True" => Ok(true),
            "no" | "false" | "False" => Ok(false),
            other => Err(ErrorKind::NotABool(other.into())),
        },
        other => Err(ErrorKind::NotABool(format!("{other:?}"))),
    }
}

fn u64_value(value: &Value) -> Result<u64> {
    match value {
        Value::Number(n) => n.as_u64().ok_or_else(|| ErrorKind::Shape {
            expected: "a non-negative integer",
            got: n.to_string(),
        }),
        other => Err(ErrorKind::Shape {
            expected: "an integer",
            got: type_name(other).into(),
        }),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "list",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(yaml: &str) -> Playbook {
        parse("test.yml", yaml).unwrap()
    }

    fn parse_err(yaml: &str) -> ErrorKind {
        parse("test.yml", yaml).unwrap_err().kind
    }

    #[test]
    fn parses_minimal_play_shape() {
        let pb = parse_ok(
            r#"
- name: Demo
  hosts: nodes
  become: yes
  vars:
    x: "{{ lookup('community.general.onepassword', 'item', field='password', vault='V') }}"
  tasks:
    - name: Ensure dir
      file:
        path: /opt/demo
        state: directory
        mode: "0755"
      register: out
    - name: Run check
      shell: eval "$(mise activate bash)" && echo hi
      args:
        executable: /bin/bash
        chdir: /opt
      changed_when: false
"#,
        );
        assert_eq!(pb.plays.len(), 1);
        let play = &pb.plays[0];
        assert!(play.becomes);
        assert_eq!(play.vars.len(), 1);
        assert_eq!(play.tasks.len(), 2);
        let TaskBody::Module(call) = &play.tasks[1].body else {
            panic!("expected module")
        };
        assert_eq!(call.module.name, "shell");
        assert!(call.free_form.as_deref().unwrap().contains("mise activate"));
        assert_eq!(call.params.len(), 2);
        assert!(matches!(
            play.tasks[1].changed_when,
            Some(Condition::Literal(false))
        ));
    }

    #[test]
    fn unknown_module_is_hard_error() {
        let kind = parse_err("- hosts: all\n  tasks:\n    - dnf:\n        name: git\n");
        assert!(matches!(kind, ErrorKind::UnknownTaskKey(k) if k == "dnf"));
    }

    #[test]
    fn unknown_param_is_hard_error() {
        let kind = parse_err(
            "- hosts: all\n  tasks:\n    - apt:\n        name: git\n        install_recommends: no\n",
        );
        assert!(
            matches!(kind, ErrorKind::UnknownParam { module, param } if module == "apt" && param == "install_recommends")
        );
    }

    #[test]
    fn literal_value_outside_surface_is_hard_error() {
        let kind = parse_err(
            "- hosts: all\n  tasks:\n    - file:\n        path: /x\n        state: touch\n",
        );
        assert!(matches!(kind, ErrorKind::ValueOutsideSurface { param, .. } if param == "state"));
    }

    #[test]
    fn templated_enum_value_passes_parse() {
        parse_ok(
            "- hosts: all\n  tasks:\n    - file:\n        path: /x\n        state: \"{{ desired }}\"\n",
        );
    }

    #[test]
    fn block_rescue_parses() {
        let pb = parse_ok(
            r#"
- hosts: all
  tasks:
    - name: Guarded
      block:
        - command: echo a
      rescue:
        - debug:
            msg: failed
"#,
        );
        let TaskBody::Block {
            block,
            rescue,
            always,
        } = &pb.plays[0].tasks[0].body
        else {
            panic!("expected block")
        };
        assert_eq!((block.len(), rescue.len(), always.len()), (1, 1, 0));
    }

    #[test]
    fn when_list_is_and() {
        let pb = parse_ok(
            "- hosts: all\n  tasks:\n    - command: echo x\n      when:\n        - a is defined\n        - a > 1\n",
        );
        assert!(matches!(
            &pb.plays[0].tasks[0].when,
            Some(Condition::All(v)) if v.len() == 2
        ));
    }

    #[test]
    fn args_on_non_shell_module_is_error() {
        let kind = parse_err(
            "- hosts: all\n  tasks:\n    - file:\n        path: /x\n        state: directory\n      args:\n        chdir: /tmp\n",
        );
        assert!(matches!(kind, ErrorKind::ArgsNotAllowed { .. }));
    }

    #[test]
    fn unknown_play_keyword_is_error() {
        let kind = parse_err("- hosts: all\n  serial: 1\n  tasks: []\n");
        assert!(matches!(kind, ErrorKind::UnknownPlayKey(k) if k == "serial"));
    }
}

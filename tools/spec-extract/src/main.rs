use ruxel_core::modules::{self, ModuleSurface};
use serde_norway::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const TASK_KEYS: &[&str] = &[
    "name",
    "when",
    "register",
    "loop",
    "loop_control",
    "vars",
    "tags",
    "notify",
    "become",
    "become_user",
    "changed_when",
    "failed_when",
    "ignore_errors",
    "check_mode",
    "no_log",
    "environment",
    "until",
    "retries",
    "delay",
    "args",
    "block",
    "rescue",
    "always",
];

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Drift {
    Module {
        file: String,
        module: String,
    },
    Param {
        file: String,
        module: String,
        param: String,
    },
    Value {
        file: String,
        module: String,
        param: String,
        value: String,
    },
    Shape {
        file: String,
        detail: String,
    },
}

fn main() {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: ruxel-spec-extract <workload-dir>");
        std::process::exit(2);
    };
    match scan(&root) {
        Ok(drift) if drift.is_empty() => {
            println!("spec-extract: no drift");
        }
        Ok(drift) => {
            for finding in drift {
                println!("{}", format_drift(&finding));
            }
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("spec-extract: {error}");
            std::process::exit(2);
        }
    }
}

fn scan(root: &Path) -> Result<BTreeSet<Drift>, String> {
    let mut files = Vec::new();
    collect_yaml(root, &mut files)?;
    files.sort();
    let mut drift = BTreeSet::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let value: Value = serde_norway::from_str(&source)
            .map_err(|error| format!("parse {}: {error}", path.display()))?;
        inspect_document(&path.display().to_string(), &value, &mut drift);
    }
    Ok(drift)
}

fn collect_yaml(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in
        std::fs::read_dir(path).map_err(|error| format!("read {}: {error}", path.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir()
            || matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            )
        {
            collect_yaml(&path, files)?;
        }
    }
    Ok(())
}

fn inspect_document(file: &str, value: &Value, drift: &mut BTreeSet<Drift>) {
    let Value::Sequence(entries) = value else {
        drift.insert(Drift::Shape {
            file: file.into(),
            detail: "root must be a sequence".into(),
        });
        return;
    };
    let looks_like_playbook = entries
        .first()
        .and_then(Value::as_mapping)
        .is_some_and(|mapping| mapping.keys().any(|key| key.as_str() == Some("hosts")));
    if looks_like_playbook {
        for play in entries {
            let Some(mapping) = play.as_mapping() else {
                continue;
            };
            for section in ["pre_tasks", "tasks", "handlers"] {
                if let Some(tasks) = map_get(mapping, section) {
                    inspect_tasks(file, tasks, drift);
                }
            }
        }
    } else {
        inspect_tasks(file, value, drift);
    }
}

fn inspect_tasks(file: &str, value: &Value, drift: &mut BTreeSet<Drift>) {
    let Some(tasks) = value.as_sequence() else {
        drift.insert(Drift::Shape {
            file: file.into(),
            detail: "tasks must be a sequence".into(),
        });
        return;
    };
    for task in tasks {
        let Some(mapping) = task.as_mapping() else {
            drift.insert(Drift::Shape {
                file: file.into(),
                detail: "task must be a mapping".into(),
            });
            continue;
        };
        for block in ["block", "rescue", "always"] {
            if let Some(children) = map_get(mapping, block) {
                inspect_tasks(file, children, drift);
            }
        }
        for (key, value) in mapping {
            let Some(module) = key.as_str() else { continue };
            if TASK_KEYS.contains(&module) {
                continue;
            }
            let Some(surface) = modules::lookup(module) else {
                drift.insert(Drift::Module {
                    file: file.into(),
                    module: module.into(),
                });
                continue;
            };
            inspect_params(file, surface, value, false, drift);
            if let Some(args) = map_get(mapping, "args") {
                inspect_params(file, surface, args, true, drift);
            }
        }
    }
}

fn inspect_params(
    file: &str,
    surface: &ModuleSurface,
    value: &Value,
    args: bool,
    drift: &mut BTreeSet<Drift>,
) {
    let Some(mapping) = value.as_mapping() else {
        return;
    };
    let allowed = if args {
        surface.args_params
    } else {
        surface.params
    };
    for (key, value) in mapping {
        let Some(param) = key.as_str() else { continue };
        if !surface.any_params && !allowed.contains(&param) {
            drift.insert(Drift::Param {
                file: file.into(),
                module: surface.name.into(),
                param: param.into(),
            });
            continue;
        }
        if let Some((_, allowed_values)) = surface
            .literal_enums
            .iter()
            .find(|(name, _)| *name == param)
            && let Some(literal) = value.as_str()
            && !literal.contains("{{")
            && !allowed_values.contains(&literal)
        {
            drift.insert(Drift::Value {
                file: file.into(),
                module: surface.name.into(),
                param: param.into(),
                value: literal.into(),
            });
        }
    }
}

fn map_get<'a>(mapping: &'a serde_norway::Mapping, key: &str) -> Option<&'a Value> {
    mapping
        .iter()
        .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
}

fn format_drift(drift: &Drift) -> String {
    match drift {
        Drift::Module { file, module } => format!("{file}: unknown module {module}"),
        Drift::Param {
            file,
            module,
            param,
        } => format!("{file}: {module}: unknown param {param}"),
        Drift::Value {
            file,
            module,
            param,
            value,
        } => {
            format!("{file}: {module}.{param}: unknown literal {value:?}")
        }
        Drift::Shape { file, detail } => format!("{file}: unsupported shape: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_workload_flags_module_param_and_value_drift() {
        let value: Value = serde_norway::from_str(
            "- hosts: all\n  tasks:\n    - apt:\n        name: git\n        unknown_option: true\n    - file:\n        path: /tmp/x\n        state: touch\n    - dnf:\n        name: git\n",
        )
        .unwrap();
        let mut drift = BTreeSet::new();
        inspect_document("synthetic.yml", &value, &mut drift);
        let output: Vec<_> = drift.iter().map(format_drift).collect();
        assert!(
            output
                .iter()
                .any(|line| line.contains("unknown module dnf"))
        );
        assert!(
            output
                .iter()
                .any(|line| line.contains("unknown param unknown_option"))
        );
        assert!(
            output
                .iter()
                .any(|line| line.contains("unknown literal \"touch\""))
        );
    }
}

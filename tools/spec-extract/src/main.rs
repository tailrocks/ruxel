use ruxel_core::modules::{self, ModuleSurface};
use serde::{Deserialize, Serialize};
use serde_norway::Value;
use std::collections::{BTreeMap, BTreeSet};
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
    "delegate_to",
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

#[derive(Debug, Default, Deserialize, Serialize)]
struct FeatureManifest {
    schema: u32,
    features: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    locations: BTreeMap<String, BTreeSet<String>>,
    #[serde(skip)]
    current_file: String,
    #[serde(skip)]
    current_task: String,
}

impl FeatureManifest {
    fn new() -> Self {
        Self {
            schema: 1,
            features: BTreeSet::new(),
            locations: BTreeMap::new(),
            current_file: String::new(),
            current_task: String::new(),
        }
    }

    fn add(&mut self, kind: &str, value: impl AsRef<str>) {
        let feature = format!("{kind}:{}", value.as_ref());
        self.features.insert(feature.clone());
        if !self.current_file.is_empty() {
            let location = if self.current_task.is_empty() {
                self.current_file.clone()
            } else {
                format!("{}#{}", self.current_file, self.current_task)
            };
            self.locations.entry(feature).or_default().insert(location);
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        eprintln!(
            "usage: ruxel-spec-extract check <workload-dir>\n       ruxel-spec-extract manifest <workload-dir> [output.json]\n       ruxel-spec-extract fixture-manifest <fixture-dir> [output.json]\n       ruxel-spec-extract coverage <workload-dir> <fixture-dir>\n       ruxel-spec-extract verify <manifest.json> <fixture-dir>"
        );
        std::process::exit(2);
    }
    let command = args[0].to_string_lossy();
    match command.as_ref() {
        "manifest" if matches!(args.len(), 2 | 3) => match extract_manifest(Path::new(&args[1])) {
            Ok(mut manifest) => {
                // The committed workload artifact is deliberately source-free.
                manifest.locations.clear();
                let json = format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap());
                if let Some(output) = args.get(2) {
                    std::fs::write(output, json).unwrap_or_else(|error| {
                        fail(&format!("write {}: {error}", Path::new(output).display()))
                    });
                } else {
                    print!("{json}");
                }
            }
            Err(error) => fail(&error),
        },
        "fixture-manifest" if matches!(args.len(), 2 | 3) => {
            match extract_manifest(Path::new(&args[1])) {
                Ok(manifest) => {
                    let json = format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap());
                    if let Some(output) = args.get(2) {
                        std::fs::write(output, json).unwrap_or_else(|error| {
                            fail(&format!("write {}: {error}", Path::new(output).display()))
                        });
                    } else {
                        print!("{json}");
                    }
                }
                Err(error) => fail(&error),
            }
        }
        "coverage" if args.len() == 3 => {
            let required =
                extract_manifest(Path::new(&args[1])).unwrap_or_else(|error| fail(&error));
            let covered =
                extract_manifest(Path::new(&args[2])).unwrap_or_else(|error| fail(&error));
            verify_coverage(&required, &covered);
        }
        "verify" if args.len() == 3 => {
            let source = std::fs::read_to_string(&args[1]).unwrap_or_else(|error| {
                fail(&format!("read {}: {error}", Path::new(&args[1]).display()))
            });
            let required: FeatureManifest = serde_json::from_str(&source).unwrap_or_else(|error| {
                fail(&format!("parse {}: {error}", Path::new(&args[1]).display()))
            });
            let covered =
                extract_manifest(Path::new(&args[2])).unwrap_or_else(|error| fail(&error));
            verify_coverage(&required, &covered);
        }
        "check" if args.len() == 2 => run_check(Path::new(&args[1])),
        _ if args.len() == 1 => run_check(Path::new(&args[0])),
        _ => fail("invalid arguments"),
    }
}

fn verify_coverage(required: &FeatureManifest, covered: &FeatureManifest) {
    if required.schema != covered.schema {
        fail(&format!(
            "manifest schema mismatch: required {}, fixture {}",
            required.schema, covered.schema
        ));
    }
    let missing = missing_features(required, covered);
    if missing.is_empty() {
        println!(
            "spec-extract: 100% fixture coverage ({} features)",
            required.features.len()
        );
    } else {
        for feature in &missing {
            println!("missing fixture feature: {feature}");
        }
        eprintln!(
            "spec-extract: {}/{} required features missing",
            missing.len(),
            required.features.len()
        );
        std::process::exit(1);
    }
}

fn missing_features<'a>(
    required: &'a FeatureManifest,
    covered: &'a FeatureManifest,
) -> Vec<&'a String> {
    required.features.difference(&covered.features).collect()
}

fn fail(error: &str) -> ! {
    eprintln!("spec-extract: {error}");
    std::process::exit(2);
}

fn run_check(root: &Path) {
    match scan(root) {
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

fn extract_manifest(root: &Path) -> Result<FeatureManifest, String> {
    let mut files = Vec::new();
    collect_yaml(root, &mut files)?;
    files.sort();
    let mut manifest = FeatureManifest::new();
    for path in files {
        manifest.current_file = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .trim_start_matches('/')
            .to_string();
        manifest.current_task.clear();
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let value: Value = serde_norway::from_str(&source)
            .map_err(|error| format!("parse {}: {error}", path.display()))?;
        collect_document_features(&value, &mut manifest);
    }
    collect_template_file_features(root, &mut manifest)?;
    Ok(manifest)
}

fn collect_template_file_features(
    path: &Path,
    manifest: &mut FeatureManifest,
) -> Result<(), String> {
    if path.is_file() {
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) {
            return Ok(());
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            return Ok(());
        };
        if metadata.len() > 1024 * 1024 {
            return Ok(());
        }
        if let Ok(text) = std::fs::read_to_string(path)
            && (text.contains("{{") || text.contains("{%"))
        {
            manifest.current_file = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("template")
                .to_string();
            manifest.current_task.clear();
            manifest.add("template-file", "jinja");
            collect_template_features("template-file", &text, manifest);
        }
        return Ok(());
    }
    for entry in
        std::fs::read_dir(path).map_err(|error| format!("read {}: {error}", path.display()))?
    {
        let child = entry.map_err(|error| error.to_string())?.path();
        if child.file_name().and_then(|value| value.to_str()) == Some(".git") {
            continue;
        }
        collect_template_file_features(&child, manifest)?;
    }
    Ok(())
}

fn collect_document_features(value: &Value, manifest: &mut FeatureManifest) {
    let Some(entries) = value.as_sequence() else {
        return;
    };
    let looks_like_playbook = entries
        .first()
        .and_then(Value::as_mapping)
        .is_some_and(|mapping| mapping.keys().any(|key| key.as_str() == Some("hosts")));
    if looks_like_playbook {
        manifest.add("document", "playbook");
        for play in entries {
            let Some(mapping) = play.as_mapping() else {
                continue;
            };
            for key in mapping.keys().filter_map(Value::as_str) {
                manifest.add("play-key", key);
            }
            for section in ["pre_tasks", "tasks", "handlers"] {
                if let Some(tasks) = map_get(mapping, section) {
                    collect_task_features(tasks, manifest);
                }
            }
            if let Some(vars) = map_get(mapping, "vars") {
                collect_value_features("play-vars", vars, manifest);
            }
        }
    } else {
        manifest.add("document", "task-list");
        collect_task_features(value, manifest);
    }
}

fn collect_task_features(value: &Value, manifest: &mut FeatureManifest) {
    let Some(tasks) = value.as_sequence() else {
        return;
    };
    for task in tasks {
        let Some(mapping) = task.as_mapping() else {
            continue;
        };
        let previous_task = manifest.current_task.clone();
        manifest.current_task = map_get(mapping, "name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)")
            .to_string();
        for block in ["block", "rescue", "always"] {
            if let Some(children) = map_get(mapping, block) {
                manifest.add("task-key", block);
                collect_value_features(&format!("task-key.{block}"), children, manifest);
                collect_task_features(children, manifest);
            }
        }
        for (key, value) in mapping {
            let Some(key) = key.as_str() else { continue };
            if TASK_KEYS.contains(&key) {
                manifest.add("task-key", key);
                collect_value_features(&format!("task-key.{key}"), value, manifest);
                continue;
            }
            manifest.add("module", key);
            collect_value_features(&format!("module.{key}"), value, manifest);
            if let Some(params) = value.as_mapping() {
                for (param, value) in params {
                    let Some(param) = param.as_str() else {
                        continue;
                    };
                    let normalized =
                        if modules::lookup(key).is_some_and(|surface| surface.any_params) {
                            "*"
                        } else {
                            param
                        };
                    manifest.add("param", format!("{key}.{normalized}"));
                    collect_value_features(&format!("param.{key}.{normalized}"), value, manifest);
                    if let Some(surface) = modules::lookup(key)
                        && surface.literal_enums.iter().any(|(name, _)| *name == param)
                        && let Some(literal) = value.as_str()
                        && !literal.contains("{{")
                    {
                        manifest.add("enum", format!("{key}.{param}={literal}"));
                    }
                }
            }
            if let Some(args) = map_get(mapping, "args").and_then(Value::as_mapping) {
                for (param, value) in args {
                    let Some(param) = param.as_str() else {
                        continue;
                    };
                    manifest.add("arg-param", format!("{key}.{param}"));
                    collect_value_features(&format!("arg-param.{key}.{param}"), value, manifest);
                }
            }
        }
        manifest.current_task = previous_task;
    }
}

fn collect_value_features(context: &str, value: &Value, manifest: &mut FeatureManifest) {
    let shape = match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(text) => {
            collect_template_features(context, text, manifest);
            if is_exact_expression(text) {
                "template-exact"
            } else if text.contains("{{") || text.contains("{%") {
                "template-mixed"
            } else {
                "string"
            }
        }
        Value::Sequence(values) => {
            for value in values {
                collect_value_features(&format!("{context}[]"), value, manifest);
            }
            "sequence"
        }
        Value::Mapping(values) => {
            for value in values.values() {
                collect_value_features(&format!("{context}.value"), value, manifest);
            }
            "mapping"
        }
        Value::Tagged(tagged) => {
            collect_value_features(context, &tagged.value, manifest);
            "tagged"
        }
    };
    manifest.add("shape", format!("{context}={shape}"));
}

fn is_exact_expression(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("{{") && trimmed.ends_with("}}") && trimmed.matches("{{").count() == 1
}

fn collect_template_features(context: &str, text: &str, manifest: &mut FeatureManifest) {
    if text.contains("{{") {
        manifest.add("template", format!("{context}:expression"));
    }
    if text.contains("{%") {
        manifest.add("template", format!("{context}:statement"));
    }
    for opener in ["{{", "{%"] {
        let closer = if opener == "{{" { "}}" } else { "%}" };
        let mut rest = text;
        while let Some(start) = rest.find(opener) {
            rest = &rest[start + opener.len()..];
            let Some(end) = rest.find(closer) else {
                break;
            };
            let expression = rest[..end].trim();
            if opener == "{%"
                && let Some(keyword) = expression.split_whitespace().next()
            {
                manifest.add("jinja-tag", keyword);
            }
            for (needle, feature) in [
                (" not in ", "not-in"),
                (" in ", "in"),
                (" and ", "and"),
                (" or ", "or"),
                ("==", "eq"),
                ("!=", "ne"),
                (">=", "ge"),
                ("<=", "le"),
                (">", "gt"),
                ("<", "lt"),
            ] {
                if expression.contains(needle) {
                    manifest.add("jinja-op", feature);
                }
            }
            if expression.contains('.') {
                manifest.add("jinja-access", "attribute");
            }
            if expression.contains('[') {
                manifest.add("jinja-access", "index");
            }
            if expression.contains('(') {
                manifest.add("jinja-expr", "call");
            }
            for segment in expression.split('|').skip(1) {
                let name: String = segment
                    .trim_start()
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .collect();
                if !name.is_empty() {
                    manifest.add("filter", name);
                }
            }
            rest = &rest[end + closer.len()..];
        }
    }
    let expression_count = text.matches("{{").count().min(3);
    if expression_count > 0 {
        manifest.add("jinja-expression-count", expression_count.to_string());
    }
    for lookup in ["community.general.onepassword", "pipe"] {
        if text.contains(lookup) {
            manifest.add("lookup", lookup);
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
        // The closed workload is the top-level playbook set. Nested YAML is
        // input data (for example application config), not Ansible tasks.
        if path.is_file()
            && !matches!(
                path.file_name().and_then(|value| value.to_str()),
                Some("requirements.yml" | "requirements.yaml")
            )
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            )
        {
            files.push(path);
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

    #[test]
    fn manifest_normalizes_private_names_and_shell_pipes() {
        let value: Value = serde_norway::from_str(
            "- hosts: all\n  vars:\n    private_name: '{{ values | default([]) }}'\n  tasks:\n    - set_fact:\n        secret_specific_name: true\n    - shell: echo x | tee /tmp/x\n",
        )
        .unwrap();
        let mut manifest = FeatureManifest::new();
        collect_document_features(&value, &mut manifest);

        assert!(manifest.features.contains("param:set_fact.*"));
        assert!(manifest.features.contains("filter:default"));
        assert!(
            !manifest
                .features
                .iter()
                .any(|entry| entry.contains("private_name"))
        );
        assert!(
            !manifest
                .features
                .iter()
                .any(|entry| entry.contains("secret_specific_name"))
        );
        assert!(!manifest.features.contains("filter:tee"));
        assert!(manifest.features.contains("jinja-expr:call"));
    }

    #[test]
    fn removed_fixture_feature_is_reported() {
        let mut required = FeatureManifest::new();
        required.add("module", "copy");
        required.add("module", "template");
        let mut covered = FeatureManifest::new();
        covered.add("module", "copy");

        assert_eq!(
            missing_features(&required, &covered),
            vec!["module:template"]
        );
    }
}

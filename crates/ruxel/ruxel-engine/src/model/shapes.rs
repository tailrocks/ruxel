use anyhow::{Result, ensure};
use serde_json::{Map, Value};

pub(super) fn validate(kind: &str, params: &Map<String, Value>, deferred: bool) -> Result<()> {
    let present = |key| params.get(key).is_some_and(|v| !v.is_null());
    if matches!(kind, "copy" | "template") {
        ensure!(
            present("src") != present("content"),
            "copy requires exactly one of src or content"
        );
    }
    if kind == "file" && params.get("state").and_then(Value::as_str) == Some("link") {
        ensure!(present("src"), "link requires src");
    }
    if kind == "file" && params.get("recurse").and_then(Value::as_bool) == Some(true) {
        ensure!(
            params.get("state").and_then(Value::as_str) == Some("directory")
                || (deferred && params.get("state").is_some_and(Value::is_object)),
            "recurse requires state=directory"
        );
    }
    if kind == "pause" {
        ensure!(
            !(present("seconds") && present("minutes")),
            "pause accepts seconds or minutes"
        );
    }
    if kind == "systemd" {
        ensure!(
            present("name") || (!present("state") && !present("enabled")),
            "service state or enabled requires name"
        );
    }
    if kind == "iptables" && present("policy") {
        ensure!(
            [
                "protocol",
                "destination",
                "out_interface",
                "jump",
                "comment"
            ]
            .iter()
            .all(|key| !present(key)),
            "firewall policy cannot contain rule fields"
        );
    }
    if kind == "lineinfile" && !(deferred && params.get("state").is_some_and(Value::is_object)) {
        if params.get("state").and_then(Value::as_str) == Some("absent") {
            ensure!(
                present("line") || present("regexp"),
                "line removal requires line or regexp"
            );
        } else {
            ensure!(present("line"), "line insertion requires line");
        }
    }
    engine_surface(kind, params)?;
    commands(kind, params)?;
    privileges(kind, params, deferred)?;
    Ok(())
}

/// The cross-field rules of the engine surface's own operations: the
/// mirrors of the constraints `Engine.pkl` carries, so a configuration
/// written directly against the documented surface is refused here too.
fn engine_surface(kind: &str, params: &Map<String, Value>) -> Result<()> {
    if kind == "apt" {
        // The mirror of the Engine surface's hold/state constraint: a
        // held package is never upgraded, so the literal pair is refused
        // at admission; an expression `hold` resolves later, where the
        // agent refuses it the same way.
        ensure!(
            !(params.get("hold") == Some(&Value::Bool(true))
                && params.get("state").and_then(Value::as_str) == Some("latest")),
            "a held package is never upgraded: hold cannot be combined with state=latest"
        );
    }
    if kind == "debconf" {
        // The mirrors of the Engine surface's `DebconfPackage` and
        // `QuestionName` constraints: an exact package name, and a
        // question without the whitespace that would shift the fields of
        // the `package question type value` line. An expression resolves
        // later, where the agent refuses it the same way.
        if let Some(name) = params.get("package").and_then(Value::as_str) {
            let mut chars = name.chars();
            let first = chars.next();
            ensure!(
                first.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    && chars.all(|c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || matches!(c, '+' | '.' | '-')),
                "a debconf selection names its package exactly"
            );
        }
        if let Some(value) = params.get("value").and_then(Value::as_str) {
            ensure!(
                !value.contains('\n') && !value.contains('\r'),
                "a debconf answer must be one line"
            );
        }
        if let Some(question) = params.get("question").and_then(Value::as_str) {
            ensure!(
                !question.is_empty()
                    && !question
                        .chars()
                        .any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')),
                "a debconf question takes no whitespace"
            );
        }
    }
    Ok(())
}

fn commands(kind: &str, params: &Map<String, Value>) -> Result<()> {
    if kind == "shell" {
        ensure!(
            params
                .get("cmd")
                .and_then(Value::as_str)
                .is_none_or(|cmd| !cmd.is_empty()),
            "command cannot be empty"
        );
    }
    if kind == "command"
        && let Some(v) = params.get("argv").filter(|v| !v.is_object())
    {
        ensure!(
            v.as_array().is_some_and(|v| !v.is_empty()),
            "argv requires nonempty list"
        );
        ensure!(
            v.as_array()
                .and_then(|v| v.first())
                .and_then(Value::as_str)
                .is_some_and(|program| !program.is_empty()),
            "command program cannot be empty"
        );
    }
    Ok(())
}

fn privileges(kind: &str, params: &Map<String, Value>, deferred: bool) -> Result<()> {
    let present = |key| params.get(key).is_some_and(|v| !v.is_null());
    if kind == "postgres_privileges"
        && !(deferred && params.get("type").is_some_and(Value::is_object))
    {
        let kind = params.get("type").and_then(Value::as_str);
        ensure!(
            kind != Some("database") || present("objs"),
            "database privileges require objs"
        );
        ensure!(
            !present("target_roles") || kind == Some("default_privs"),
            "target roles require default privileges"
        );
    }

    Ok(())
}

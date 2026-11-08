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
    commands(kind, params)?;
    privileges(kind, params, deferred)?;
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

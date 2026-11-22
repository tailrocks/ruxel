use super::*;

fn workflow(operation: Value) -> Value {
    let mut config = json!({"inventory":{"hosts":{"web":{"address":"unused.invalid"},"db":{"address":"unused.invalid"}}},"plays":[{"name":"base","targets":["web"],"tasks":[{"name":"test"}]},{"name":"app","targets":["db"],"tasks":[{"name":"test"}]}]});
    config["plays"][0]["tasks"][0]["operation"] = operation.clone();
    config["plays"][1]["tasks"][0]["operation"] = operation;
    config
}

#[test]
fn namespace_is_stable_over_task_edits() {
    // Thread 304: the namespace names the workflow, not its content —
    // a task edit must not fork the ledger identity, or the edited
    // run orphans every prior record and replays the multi-stage
    // hazard for one transition.
    let before = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let after = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "two"}))).expect("config"),
    );
    assert_eq!(before, after, "a task edit keeps the workflow's identity");
}

#[test]
fn namespace_ignores_play_and_host_order() {
    // The derivation sorts what it names: order is presentation, not
    // identity.
    let forward = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let mut config = workflow(json!({"kind": "debug", "msg": "one"}));
    let plays: Vec<Value> = config["plays"]
        .as_array()
        .expect("plays")
        .clone()
        .into_iter()
        .rev()
        .collect();
    config["plays"] = Value::Array(plays);
    let backward = workflow_namespace(&validate_config(&config).expect("config"));
    assert_eq!(forward, backward, "play order is not identity");
}

#[test]
fn namespace_names_plays_and_hosts() {
    // Distinct workflows sharing nothing but shape own distinct
    // namespaces: the identity covers the play names and the
    // inventory host names.
    let base = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let mut renamed = workflow(json!({"kind": "debug", "msg": "one"}));
    renamed["plays"][0]["name"] = json!("other");
    let renamed = workflow_namespace(&validate_config(&renamed).expect("config"));
    assert_ne!(base, renamed, "a play rename is a different workflow");
    let mut rehosted = workflow(json!({"kind": "debug", "msg": "one"}));
    rehosted["inventory"]["hosts"] = json!({"elsewhere": {"address": "unused.invalid"}});
    rehosted["plays"][0]["targets"] = json!(["elsewhere"]);
    rehosted["plays"][1]["targets"] = json!(["elsewhere"]);
    let rehosted = workflow_namespace(&validate_config(&rehosted).expect("config"));
    assert_ne!(base, rehosted, "a host rename is a different workflow");
}

#[test]
fn namespace_is_opaque() {
    // Threads 304 and 306: the namespace travels to the remote agent
    // in every Hello, so it carries no path metadata — a hex digest,
    // never a filesystem location.
    let namespace = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    assert!(
        namespace.chars().all(|c| c.is_ascii_hexdigit()),
        "the namespace is a hex digest, not a path: {namespace}"
    );
    assert!(!namespace.contains('/'), "no path separator: {namespace}");
    assert!(!namespace.contains('.'), "no path metadata: {namespace}");
}

use super::*;

fn workflow(operation: Value) -> Value {
    let mut config = json!({"workflow":"synthetic-workflow","inventory":{"hosts":{"web":{"address":"unused.invalid"},"db":{"address":"unused.invalid"}}},"plays":[{"name":"base","targets":["web"],"tasks":[{"name":"test"}]},{"name":"app","targets":["db"],"tasks":[{"name":"test"}]}]});
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
fn namespace_names_the_workflow() {
    // Threads 319 and 324: the explicit workflow identity is the
    // namespace's first domain — two workflows that reuse every play
    // and host name own distinct namespaces, so neither one's records
    // can enter the other's managed set, and neither run prunes the
    // other's cache and declarations.
    let base = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let mut renamed = workflow(json!({"kind": "debug", "msg": "one"}));
    renamed["workflow"] = json!("another-workflow");
    let renamed = workflow_namespace(&validate_config(&renamed).expect("config"));
    assert_ne!(base, renamed, "a workflow rename is a different owner");
}

#[test]
fn namespace_names_the_play_to_host_mapping() {
    // Thread 319: the preimage carries (play, sorted targets) pairs, not
    // two flat name lists — swapping which play runs on which host is a
    // different workflow, the way renaming either side is.
    let base = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let mut swapped = workflow(json!({"kind": "debug", "msg": "one"}));
    swapped["plays"][0]["targets"] = json!(["db"]);
    swapped["plays"][1]["targets"] = json!(["web"]);
    let swapped = workflow_namespace(&validate_config(&swapped).expect("config"));
    assert_ne!(base, swapped, "a play-to-host swap is a different workflow");
}

#[test]
fn namespace_has_no_cross_domain_collisions() {
    // Thread 324: every part is length-prefixed, tagged and counted, so
    // no part list reads as another — plays ["a", "b"] never hash like
    // one play "a\0b" under the old flat join, and a workflow name never
    // reads as a play name.
    let base = workflow_namespace(
        &validate_config(&workflow(json!({"kind": "debug", "msg": "one"}))).expect("config"),
    );
    let mut merged = workflow(json!({"kind": "debug", "msg": "one"}));
    merged["plays"] = json!([{"name": "base\0app", "targets": [], "tasks": [{"name": "test", "operation": {"kind": "debug"}}]}]);
    let merged = workflow_namespace(&validate_config(&merged).expect("config"));
    assert_ne!(base, merged, "a merged play name is a different workflow");
    let mut smuggled = workflow(json!({"kind": "debug", "msg": "one"}));
    smuggled["workflow"] = json!("synthetic-workflow\0plays:2");
    let smuggled = workflow_namespace(&validate_config(&smuggled).expect("config"));
    assert_ne!(base, smuggled, "a workflow name cannot smuggle structure");
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

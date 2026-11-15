use super::*;

#[test]
fn a_sysctl_record_of_the_old_probe_schema_is_not_served() {
    // The parent agent's sysctl record carries the key/value probe and
    // the live probe but no boot probe, and it identifies itself as
    // this agent's version: the record deserializes and verifies, so
    // it is served from the cache — and the task it belongs to is
    // skipped, and with it the override preflight the boot probe
    // guards. A record of the old schema must not be served.
    let dir = Scratch::new("sysctl-old-schema");
    let file = dir.file("sysctl.conf", b"net.test = 1\n");
    let ledger = Ledger::load(&dir.0);
    let key = ledger
        .storage_key("controller-key")
        .expect("the host key is there");
    let old = json!({
        key: {
            "agent_version": env!("CARGO_PKG_VERSION"),
            "status": "ok",
            "result_json": {"changed": false},
            "probes": [{
                "kind": "SysctlKV",
                "file": file.to_str().expect("operation succeeds"),
                "name": "net.test",
                "value": "1"
            }]
        }
    });
    std::fs::write(
        dir.0.join("ledger/ledger.json"),
        serde_json::to_vec(&old).expect("operation succeeds"),
    )
    .expect("operation succeeds");
    assert!(
        Ledger::load(&dir.0).cached_ok("controller-key").is_none(),
        "a sysctl record without the boot probe is of the old schema and must not be served"
    );
}

#[test]
fn a_sysctl_declaration_survives_a_file_the_probe_cannot_read() {
    // A file that assigns the key twice is no cacheable result: the
    // probe reads the file as text and refuses the duplicate. The
    // declaration the task made of it is the workflow's own all the
    // same — the preflight must weigh the file as managed, or it
    // refuses the earlier stage that the later stage's declaration
    // belongs to.
    let dir = Scratch::new("sysctl-declaration");
    let file = dir.file("sysctl.conf", b"net.test = 1\nnet.test = 1\n");
    let params = json!({"name": "net.test", "value": "1", "sysctl_file": file});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("key", "sysctl", &params, "ok", &json!({"changed": true}));
    assert!(
        ledger.cached_ok("key").is_none(),
        "the duplicate assignment is no cacheable result"
    );
    assert!(
        ledger.managed_sysctl_entries().contains(&(
            file.to_str().expect("operation succeeds").to_string(),
            "net.test".to_string(),
            "1".to_string()
        )),
        "the declaration of the task must survive: {:?}",
        ledger.managed_sysctl_entries()
    );
}

#[test]
fn a_managed_declaration_does_not_need_a_cacheable_probe() {
    // The declaration is the task's own, not the file's content: a
    // file whose bytes are not text is no cacheable result either,
    // and the declaration of the key in it is still the workflow's.
    let dir = Scratch::new("sysctl-binary-declaration");
    let file = dir.file("sysctl.conf", b"net.test = 1\n\xff\xfe not text\n");
    let params = json!({"name": "net.test", "value": "1", "sysctl_file": file});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("key", "sysctl", &params, "ok", &json!({"changed": true}));
    assert!(ledger.cached_ok("key").is_none());
    assert_eq!(
        ledger.managed_sysctl_entries(),
        vec![(
            file.to_str().expect("operation succeeds").to_string(),
            "net.test".to_string(),
            "1".to_string()
        )]
    );
}

#[test]
fn an_aborted_plan_keeps_the_records_of_the_tasks_it_never_reached() {
    // A task fails and the plan halts on the failure: the tasks it
    // never reached are still the workflow's, and their records are
    // not the history of a task the workflow dropped. The next run
    // must see the earlier stage's declaration of a later task, or it
    // treats the later task's file as an unmanaged override and
    // refuses the earlier stage before the later task can run.
    let dir = Scratch::new("aborted-plan");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "later", &file);
    ledger.flush();

    let mut ledger = Ledger::load(&dir.0);
    ledger.encounter("earlier");
    ledger.complete_plan();
    ledger.abort_plan();
    ledger.flush();

    assert!(
        Ledger::load(&dir.0).cached_ok("later").is_some(),
        "the record of the task the aborted plan never reached must survive"
    );
}

#[test]
fn a_complete_plan_expires_the_records_it_never_encountered() {
    // The counterpart: a plan that ran every task of its workflow is
    // that workflow's verdict on what it manages, and the records of
    // the tasks it does not run anymore are history.
    let dir = Scratch::new("complete-plan");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "later", &file);
    ledger.flush();

    let mut ledger = Ledger::load(&dir.0);
    ledger.encounter("earlier");
    ledger.complete_plan();
    ledger.flush();

    assert!(
        Ledger::load(&dir.0).cached_ok("later").is_none(),
        "the record of a task the plan dropped must expire"
    );
}

#[test]
fn an_unfinished_session_expires_nothing() {
    // The controller's stream closed before the end of the session: the
    // run is an executed prefix, not a complete plan, so the records
    // of the tasks it never reached are the workflow's own and must
    // not expire.
    let dir = Scratch::new("unfinished-session");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "later", &file);
    ledger.flush();

    let mut ledger = Ledger::load(&dir.0);
    ledger.encounter("earlier");
    ledger.flush();

    assert!(
        Ledger::load(&dir.0).cached_ok("later").is_some(),
        "an unfinished session must expire nothing"
    );
}

//! A sensitive task earlier in the session forces every later task
//! to `no_log`: the sysctl declaration still records — the
//! file/name/value triple the workflow manages — while the cached
//! result and the probes stay out of the ledger as before.

use serde_json::json;

use super::super::boot;

/// The declaration-only record of a `no_log` sysctl task: what
/// `serve` stores when the session is sensitive.
fn declare_no_log(
    ledger: &mut crate::ledger::Ledger,
    key: &str,
    name: &str,
    value: &str,
    file: &std::path::Path,
) {
    let params = json!({
        "name": name,
        "value": value,
        "sysctl_file": file.to_string_lossy(),
    });
    ledger.record_managed_declaration(key, "sysctl", &params, "ok");
}

#[test]
fn a_no_log_sysctl_still_declares_its_managed_entry() {
    // A sensitive task forced the session to `no_log`, then two
    // stages set one key in `10-a.conf` and to another value in
    // `99-b.conf`: both applies succeed, and on the next run the
    // earlier stage must still pass — the later file holds the
    // workflow's own declared transition, not an unmanaged override.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-no-log-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let first = dir.join("10-a.conf");
    let second = dir.join("99-b.conf");
    std::fs::write(&first, "net.test=1\n").expect("operation succeeds");
    std::fs::write(&second, "net.test=2\n").expect("operation succeeds");
    let first_text = first.to_string_lossy().into_owned();
    let second_text = second.to_string_lossy().into_owned();
    let mut ledger = crate::ledger::Ledger::load(&dir);
    declare_no_log(&mut ledger, "first", "net.test", "1", &first);
    declare_no_log(&mut ledger, "second", "net.test", "2", &second);
    let mut managed = ledger.managed_sysctl_entries();
    managed.sort();
    assert_eq!(
        managed,
        vec![
            (first_text.clone(), "net.test".to_string(), "1".to_string()),
            (second_text.clone(), "net.test".to_string(), "2".to_string()),
        ],
        "both stages declare their entries despite no_log"
    );
    // The earlier stage runs again: the later file's assignment is
    // the declared managed value of the workflow's own later stage.
    let host = vec![(second.clone(), b"net.test=2\n".to_vec())];
    assert_eq!(
        boot::boot_override(&first_text, "net.test", "1", &host, &managed,),
        None,
        "the declared later stage is a transition, not an unmanaged override"
    );
    // Nothing cached: neither task serves a result from the ledger.
    assert!(
        ledger.cached_ok("first").is_none(),
        "a declaration-only record must not serve cached results"
    );
    assert!(
        ledger.cached_ok("second").is_none(),
        "a declaration-only record must not serve cached results"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_no_log_record_holds_no_result_and_no_probes() {
    // Suppression is the point of `no_log`: the declaration lands
    // without anything the task observed.
    let dir =
        std::env::temp_dir().join(format!("ruxel-sysctl-no-log-quiet-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let file = dir.join("10-a.conf");
    std::fs::write(&file, "net.test=1\n").expect("operation succeeds");
    let mut ledger = crate::ledger::Ledger::load(&dir);
    declare_no_log(&mut ledger, "first", "net.test", "1", &file);
    assert_eq!(
        ledger.managed_sysctl_entries(),
        vec![(
            file.to_string_lossy().into_owned(),
            "net.test".to_string(),
            "1".to_string()
        )],
    );
    assert!(ledger.cached_ok("first").is_none());
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_failed_or_unrelated_no_log_task_declares_nothing() {
    // A task that failed wrote nothing, and a task of another module
    // manages no sysctl: neither leaves a declaration behind.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-no-log-void-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let file = dir.join("10-a.conf");
    let mut ledger = crate::ledger::Ledger::load(&dir);
    let params = json!({
        "name": "net.test",
        "value": "1",
        "sysctl_file": file.to_string_lossy(),
    });
    ledger.record_managed_declaration("failed", "sysctl", &params, "failed");
    ledger.record_managed_declaration("other", "file", &params, "ok");
    ledger.record_managed_declaration("", "sysctl", &params, "ok");
    assert_eq!(
        ledger.managed_sysctl_entries(),
        Vec::<(String, String, String)>::new(),
        "no declaration without a successful sysctl task"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_no_log_declaration_keeps_an_existing_record_cached() {
    // The declaration lands on the record the task already holds: its
    // probes and result survive, so a cached task stays cached while
    // its declaration moves to the latest value.
    let dir =
        std::env::temp_dir().join(format!("ruxel-sysctl-no-log-merge-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let file = dir.join("10-a.conf");
    std::fs::write(&file, "net.test=1\n").expect("operation succeeds");
    let mut ledger = crate::ledger::Ledger::load(&dir);
    let params = json!({
        "name": "net.test",
        "value": "1",
        "sysctl_file": file.to_string_lossy(),
    });
    ledger.record(
        "first",
        "sysctl",
        &params,
        "ok",
        &json!({"changed": true, "failed": false}),
    );
    assert!(
        ledger.cached_ok("first").is_some(),
        "the recorded task serves from the cache"
    );
    let params = json!({
        "name": "net.test",
        "value": "2",
        "sysctl_file": file.to_string_lossy(),
    });
    ledger.record_managed_declaration("first", "sysctl", &params, "ok");
    // The declaration moves to the latest value while the probed
    // entry the file still holds stays beside it: entries union the
    // declared and the probed, and the preflight weighs both.
    assert!(
        ledger.managed_sysctl_entries().contains(&(
            file.to_string_lossy().into_owned(),
            "net.test".to_string(),
            "2".to_string()
        )),
        "the declaration moves to the latest value"
    );
    assert!(
        ledger.cached_ok("first").is_some(),
        "the existing record stays cached"
    );
    drop(std::fs::remove_dir_all(&dir));
}

//! A sensitive task earlier in the session forces every later task
//! to `no_log`: the sysctl declaration still records — the
//! file/name/value triple the workflow manages — while the cached
//! result and the probes stay out of the ledger as before.

use serde_json::json;

use super::super::{ManagedSysctl, ManagedValue, boot};

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
    // The preflight's managed set is plan-first: the frames carry the
    // full static set, so the plan's own plain declarations lead and
    // the ledger's sealed records of the same keys stay behind them —
    // the shape `serve` consults. The seals themselves are weighed
    // where the ledger's own set is read.
    let managed = ledger.sysctl_preflight_managed(
        "sysctl",
        &[
            ManagedSysctl::plain(first_text.clone(), "net.test".to_string(), "1".to_string()),
            ManagedSysctl::plain(second_text.clone(), "net.test".to_string(), "2".to_string()),
        ],
    );
    assert_eq!(
        managed.entries.len(),
        2,
        "both stages declare their entries despite no_log"
    );
    assert_eq!(
        managed.plan.len(),
        2,
        "the plan's own declarations authorize the waiver"
    );
    for entry in ledger.managed_sysctl_entries() {
        assert!(
            matches!(entry.value, ManagedValue::Sealed(_)),
            "a declaration-only record keeps only the value's seal: {}",
            entry.file
        );
    }
    assert!(
        managed
            .entries
            .iter()
            .any(|entry| entry.file == first_text && entry.name == "net.test"),
        "the earlier stage's entry is retained"
    );
    assert!(
        managed
            .entries
            .iter()
            .any(|entry| entry.file == second_text && entry.name == "net.test"),
        "the later stage's entry is retained"
    );
    // The earlier stage runs again: the later file's assignment is
    // the declared managed value of the workflow's own later stage.
    let host = vec![(second.clone(), b"net.test=2\n".to_vec())];
    assert_eq!(
        boot::boot_override(&first_text, "net.test", "1", &host, &managed,),
        None,
        "the declared later stage is a transition, not an unmanaged override"
    );
    // A drifted later file is no declared value: the seal binds the
    // declared bytes, so tampering refuses instead of authorizing.
    let tampered = vec![(second.clone(), b"net.test=3\n".to_vec())];
    assert!(
        boot::boot_override(&first_text, "net.test", "1", &tampered, &managed,).is_some(),
        "a tampered later file is an unmanaged override again"
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
    declare_no_log(&mut ledger, "first", "net.test", "s3cr3t-gamma", &file);
    let entries = ledger.managed_sysctl_entries();
    assert_eq!(entries.len(), 1, "the declaration is retained");
    let entry = &entries[0];
    assert_eq!(entry.file, file.to_string_lossy());
    assert_eq!(entry.name, "net.test");
    assert!(
        matches!(entry.value, ManagedValue::Sealed(_)),
        "the declaration keeps only the value's seal"
    );
    assert!(ledger.cached_ok("first").is_none());
    // The value itself never reaches the disk.
    ledger.flush();
    let stored = std::fs::read_to_string(dir.join("ledger").join("ledger.json"))
        .expect("operation succeeds");
    assert!(
        !stored.contains("s3cr3t-gamma"),
        "the ledger file must not carry the no_log value"
    );
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
    assert_eq!(
        ledger.managed_sysctl_entries(),
        Vec::<ManagedSysctl>::new(),
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
    // declared and the probed, and the preflight weighs both. The
    // declaration is sealed — the value is never stored — while the
    // probe keeps the file's observed bytes.
    let entries = ledger.managed_sysctl_entries();
    assert!(
        entries
            .iter()
            .any(|entry| entry.file == file.to_string_lossy()
                && entry.name == "net.test"
                && matches!(entry.value, ManagedValue::Sealed(_))),
        "the declaration moves to the latest value, sealed"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.file == file.to_string_lossy()
                && entry.name == "net.test"
                && matches!(&entry.value, ManagedValue::Plain(value) if value == "1")),
        "the probed entry stays beside it"
    );
    assert!(
        ledger.cached_ok("first").is_some(),
        "the existing record stays cached"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn an_empty_key_no_log_sysctl_declares_without_storing_its_value() {
    // Thread 308/315: the controller sends an empty ledger key for a
    // no_log task, so the declaration must live under its own stable
    // secret-safe key — and the value itself must never reach the disk.
    let dir =
        std::env::temp_dir().join(format!("ruxel-sysctl-no-log-empty-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let first = dir.join("10-a.conf");
    let second = dir.join("99-b.conf");
    std::fs::write(&first, "net.test=s3cr3t-alpha\n").expect("operation succeeds");
    std::fs::write(&second, "net.test=s3cr3t-beta\n").expect("operation succeeds");
    let first_text = first.to_string_lossy().into_owned();
    let second_text = second.to_string_lossy().into_owned();
    let mut ledger = crate::ledger::Ledger::load(&dir);
    let secret = "s3cr3t-beta";
    let params = serde_json::json!({
        "name": "net.test",
        "value": secret,
        "sysctl_file": second_text,
    });
    ledger.record_managed_declaration("", "sysctl", &params, "ok");
    let entries = ledger.managed_sysctl_entries();
    assert!(
        entries
            .iter()
            .any(|entry| entry.file == second_text && entry.name == "net.test"),
        "the empty-key declaration is retained for the later stage's file"
    );
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry.value, ManagedValue::Sealed(_))),
        "the retained declaration keeps only the value's seal"
    );
    // The earlier stage still passes: the plan declares the later
    // file's value, so it holds the workflow's own declared
    // transition, not an unmanaged override.
    let managed = ledger.sysctl_preflight_managed(
        "sysctl",
        &[ManagedSysctl::plain(
            second_text.clone(),
            "net.test".to_string(),
            secret.to_string(),
        )],
    );
    let host = vec![(second.clone(), b"net.test=s3cr3t-beta\n".to_vec())];
    assert_eq!(
        boot::boot_override(&first_text, "net.test", "s3cr3t-alpha", &host, &managed,),
        None,
        "the retained declaration authorizes the later stage's value"
    );
    // The value itself never reaches the disk.
    ledger.flush();
    let stored = std::fs::read_to_string(dir.join("ledger").join("ledger.json"))
        .expect("operation succeeds");
    assert!(
        !stored.contains(secret),
        "the ledger file must not carry the no_log value"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn an_empty_key_declaration_survives_a_second_apply() {
    // Thread 308: the second apply replays with fresh agent state, so
    // the declaration must be readable from a reloaded ledger — without
    // it the earlier stage refuses the later file before the later
    // task can run.
    let dir =
        std::env::temp_dir().join(format!("ruxel-sysctl-no-log-reload-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let second = dir.join("99-b.conf");
    let second_text = second.to_string_lossy().into_owned();
    {
        let mut ledger = crate::ledger::Ledger::load(&dir);
        let params = serde_json::json!({
            "name": "net.test",
            "value": "s3cr3t-beta",
            "sysctl_file": second_text,
        });
        ledger.record_managed_declaration("", "sysctl", &params, "ok");
        ledger.complete_plan();
        ledger.flush();
    }
    let reloaded = crate::ledger::Ledger::load(&dir);
    let entries = reloaded.managed_sysctl_entries();
    assert!(
        entries
            .iter()
            .any(|entry| entry.file == second_text && entry.name == "net.test"),
        "the declaration survives the reload of the next apply"
    );
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry.value, ManagedValue::Sealed(_))),
        "the reloaded declaration keeps only the value's seal"
    );
    drop(std::fs::remove_dir_all(&dir));
}

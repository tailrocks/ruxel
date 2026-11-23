//! The dropped-task replay through the cache: run 1 refreshes the
//! earlier task's record while the removed later task's retained
//! declaration still authorizes its file; the plan's end prunes the
//! removed task; run 2 must not serve the earlier task from the cache.
//! The file is untouched and the boot surface is byte-identical, so
//! every probe still matches — the fingerprint pins bytes, not who
//! manages them — and only the declaration set tells the cache is
//! stale. Without it the cache-hit path bypasses the override
//! preflight forever, reporting convergence while reboot restores the
//! removed task's value.

use super::*;

const KEY: &str = "net.test";
const LATE: &str = "/etc/sysctl.d/99-old.conf";

/// The boot surface the record is replayed over: the dropped task's
/// leftover assigns another value from the later-loading file. The
/// earlier task's own file lives in the scratch dir and is untouched
/// throughout, so its probes match in both runs.
fn host() -> Vec<(std::path::PathBuf, Vec<u8>)> {
    vec![(std::path::PathBuf::from(LATE), b"net.test = 0\n".to_vec())]
}

#[test]
fn a_dropped_sysctl_task_invalidates_the_earlier_tasks_cache() {
    let dir = Scratch::new("sysctl-declaration-drift");
    let early = dir.file("10-new.conf", b"net.test = 1\n");
    let early_text = early.to_str().expect("path is text").to_owned();
    let params = json!({"name": KEY, "value": "1", "sysctl_file": early_text.clone()});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("early", "sysctl", &params, "ok", &json!({"changed": true}));

    // The record exists and its probes match: the file is untouched
    // and the host surface moved nowhere.
    assert!(ledger.cached_ok("early").is_some());

    // Run 1: the removed task's retained declaration still authorizes
    // its file — the one-run bounded window — so the cache serves.
    let retained = crate::modules::SysctlManaged::testing(vec![
        crate::modules::ManagedSysctl::plain(early_text.clone(), KEY.to_string(), "1".to_string()),
        crate::modules::ManagedSysctl::plain(LATE.to_string(), KEY.to_string(), "0".to_string()),
    ]);
    assert!(
        ledger
            .cached_ok_with_managed_in(
                "early",
                &mut crate::system_state::SystemState::default(),
                &retained,
                &host()
            )
            .is_some(),
        "run 1 serves the earlier task: the retained declaration still authorizes the leftover"
    );

    // Run 2: the plan's end pruned the removed task, so its leftover
    // is unmanaged. Every probe still matches, yet the cache must miss:
    // the boot value comes from the leftover now, and the task must
    // run so its preflight refuses it.
    let pruned =
        crate::modules::SysctlManaged::testing(vec![crate::modules::ManagedSysctl::plain(
            early_text,
            KEY.to_string(),
            "1".to_string(),
        )]);
    assert!(
        ledger
            .cached_ok_with_managed_in(
                "early",
                &mut crate::system_state::SystemState::default(),
                &pruned,
                &host()
            )
            .is_none(),
        "run 2 must miss: the dropped task's leftover is unmanaged and restores its value at boot"
    );
}

#[test]
fn a_non_sysctl_record_ignores_the_managed_set() {
    // The gate weighs only sysctl records: any other module's cache is
    // decided by its probes alone, whatever declarations are current.
    let dir = Scratch::new("sysctl-gate-scope");
    let target = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &target);
    assert!(
        ledger
            .cached_ok_with_managed_in(
                "key",
                &mut crate::system_state::SystemState::default(),
                &crate::modules::SysctlManaged::default(),
                &host()
            )
            .is_some(),
        "a file record stands on its probes alone"
    );
}

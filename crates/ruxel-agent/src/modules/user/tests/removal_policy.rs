//! Removals against the host's system-account policy: the boundary
//! comes from `/etc/login.defs`, not the fixed 1000 — and an unreadable
//! policy fails the removal closed.

use super::super::*;

const DEBIAN_POLICY: &str = "SYS_UID_MIN 100\nSYS_UID_MAX 999\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n";
const WIDE_POLICY: &str = "SYS_UID_MIN 100\nSYS_UID_MAX 1999\nSYS_GID_MIN 100\nSYS_GID_MAX 1999\n";

#[test]
fn a_system_account_is_never_removed_whatever_the_workflow_states() {
    // The daemon scenario: the workflow states uid 1000, the host
    // account is UID 1. The host entry, not the workflow value, is
    // what the deletion acts on. The pinned policy keeps the boundary
    // provable without the host's own `/etc/login.defs`.
    let _policy = super::super::guards::pin_test_policy(DEBIAN_POLICY);
    let daemon = PasswdEntry {
        uid: 1,
        gid: 1,
        ..PasswdEntry::default()
    };
    assert!(removal_allowed("daemon", Some(1000), Some(&daemon), false).is_err());
    assert!(removal_allowed("daemon", None, Some(&daemon), false).is_err());
    let toor = PasswdEntry {
        uid: 0,
        ..PasswdEntry::default()
    };
    assert!(removal_allowed("toor", Some(0), Some(&toor), false).is_err());
    let app = PasswdEntry {
        uid: 1500,
        ..PasswdEntry::default()
    };
    // Thread 336: the boundary is the host's configured system range,
    // not the fixed 1000 — a uid above 1000 is the host's to keep
    // where the policy's ceiling covers it, and a login uid is
    // removable where it does not.
    assert!(removal_allowed("app", Some(1500), Some(&app), false).is_ok());
    assert!(removal_allowed("app", Some(1000), Some(&app), false).is_err());
    assert!(removal_allowed("app", None, Some(&app), false).is_ok());
    let _policy = super::super::guards::pin_test_policy(WIDE_POLICY);
    let error = removal_allowed("app", Some(1500), Some(&app), false)
        .expect_err("a uid inside the host's system range must not be removed");
    assert!(error.contains("system account"), "{error}");
    // Thread 326: a declared system account is the host's to keep even
    // where the allocator drew it a regular-looking uid — the live
    // entry carries no system bit, so the plan's own record decides.
    assert!(removal_allowed("app", Some(1500), Some(&app), true).is_err());
    assert!(removal_allowed("app", None, Some(&app), true).is_err());
    let error = removal_allowed("app", Some(1500), Some(&app), true)
        .expect_err("a declared system account must not be removed");
    assert!(error.contains("system account"), "{error}");
    // With no entry at all nothing verifies — unless the plan declared
    // a system account, which is refused on the declaration alone.
    assert!(removal_allowed("app", None, None, false).is_ok());
    assert!(removal_allowed("app", None, None, true).is_err());
    let _policy = super::super::guards::pin_test_policy(DEBIAN_POLICY);
    assert!(group_removal_allowed("app", Some(1500), 1500, None).is_ok());
    assert!(group_removal_allowed("app", Some(1000), 1500, None).is_err());
    let _policy = super::super::guards::pin_test_policy(WIDE_POLICY);
    let error = group_removal_allowed("app", Some(1500), 1500, None)
        .expect_err("a gid inside the host's system range must not be removed");
    assert!(error.contains("system group"), "{error}");
    assert!(group_removal_allowed("wheel", None, 0, None).is_err());
    // A group a live account holds as its primary group is the
    // account's to keep: `groupdel` refuses to delete one without
    // `--force`, which this removal does not pass.
    assert!(group_removal_allowed("app", Some(1500), 1500, Some("alice")).is_err());
}

#[test]
fn an_unreadable_host_policy_fails_a_removal_closed() {
    // Thread 336: where the host names no usable system range — the
    // file is missing, or a key is missing or unparsable — the removal
    // is refused instead of treating the number as a removable login
    // account. The pinned content exercises the full fail-closed path:
    // the pin, the parse and the refusal mapping.
    let app = PasswdEntry {
        uid: 1500,
        ..PasswdEntry::default()
    };
    for policy in [
        "",
        "SYS_UID_MIN 100\n",
        "SYS_UID_MIN many\nSYS_UID_MAX 999\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n",
    ] {
        let _policy = super::super::guards::pin_test_policy(policy);
        let error = removal_allowed("app", Some(1500), Some(&app), false)
            .expect_err("an unusable host policy must fail the removal closed");
        assert!(error.contains("system-account range"), "{error}");
        let error = group_removal_allowed("app", Some(1500), 1500, None)
            .expect_err("an unusable host policy must fail the removal closed");
        assert!(error.contains("system-account range"), "{error}");
    }
}

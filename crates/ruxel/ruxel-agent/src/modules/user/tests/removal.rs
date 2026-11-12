//! Removal verifies the host entry, not the workflow value: the
//! superuser's account and system accounts are the host's to keep, and a
//! declared id must match the host's.

use super::super::*;

#[test]
fn removal_verifies_the_host_entry_not_the_workflow_value() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // root is uid 0 on every host: a workflow uid of 1000 must not
    // authorize removing the superuser's account.
    let error = run(&json!({"name":"root", "state":"absent", "uid":1000}), &ctx)
        .expect_err("removal must verify the host uid, not the workflow value");
    assert!(error.contains("refusing to remove root"), "{error}");
    // Without a declared uid the host entry alone decides, and uid 0
    // is kept whatever the account is named.
    assert!(run(&json!({"name":"root", "state":"absent"}), &ctx).is_err());
}

#[test]
fn group_removal_verifies_the_host_entry_not_the_workflow_value() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The GID-0 group's name is a host detail (root, wheel, …): a
    // workflow gid of 1500 must not authorize removing it whatever
    // it is called.
    let host = std::fs::read_to_string("/etc/group").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2] == "0").then(|| f[0].to_string())
        })
        .expect("a gid-0 group");
    let error = group(&json!({"name":name, "state":"absent", "gid":1500}), &ctx)
        .expect_err("group removal must verify the host gid, not the workflow value");
    assert!(error.contains("refusing to remove"), "{error}");
    assert!(group(&json!({"name":name, "state":"absent"}), &ctx).is_err());
}

#[test]
fn a_system_account_is_never_removed_whatever_the_workflow_states() {
    // The daemon scenario: the workflow states uid 1000, the host
    // account is UID 1. The host entry, not the workflow value, is
    // what the deletion acts on.
    let daemon = PasswdEntry {
        uid: 1,
        gid: 1,
        ..PasswdEntry::default()
    };
    assert!(removal_allowed("daemon", Some(1000), &daemon).is_err());
    assert!(removal_allowed("daemon", None, &daemon).is_err());
    let toor = PasswdEntry {
        uid: 0,
        ..PasswdEntry::default()
    };
    assert!(removal_allowed("toor", Some(0), &toor).is_err());
    let app = PasswdEntry {
        uid: 1500,
        ..PasswdEntry::default()
    };
    assert!(removal_allowed("app", Some(1500), &app).is_ok());
    assert!(removal_allowed("app", Some(1000), &app).is_err());
    assert!(removal_allowed("app", None, &app).is_ok());
    assert!(group_removal_allowed("app", Some(1500), 1500).is_ok());
    assert!(group_removal_allowed("app", Some(1000), 1500).is_err());
    assert!(group_removal_allowed("wheel", None, 0).is_err());
}

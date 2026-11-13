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

#[test]
fn a_recursive_removal_verifies_the_declared_home_against_the_live_entry() {
    // `userdel -r` deletes the LIVE home and mail spool whatever the
    // workflow names: a declared home that is not the live one reserves
    // the wrong path in the lowering while the deletion takes another —
    // a same-stage file beneath the live home would be deleted though no
    // check reserved it.
    let app = PasswdEntry {
        uid: 1500,
        gid: 1500,
        home: "/home/app".to_string(),
        ..PasswdEntry::default()
    };
    assert!(removal_home_allowed("app", Some("/home/app"), true, &app.home).is_ok());
    assert!(removal_home_allowed("app", None, true, &app.home).is_err());
    assert!(
        removal_home_allowed("app", Some("/srv/app"), true, &app.home).is_err(),
        "a recursive removal must not delete a home the workflow never declared"
    );
    // Without removeHome nothing is deleted, so any declared home is fine.
    assert!(removal_home_allowed("app", Some("/srv/app"), false, &app.home).is_ok());
}

#[test]
fn a_removal_declares_the_live_primary_group() {
    // The lowering derives the removal's ordering from the declared
    // primary group — the private-group default for an account that
    // names none — which holds only where the declaration matches the
    // live entry. An account originally created with another primary
    // group makes the default wrong: the removal refuses and asks for
    // the real group to be stated, before `userdel` mutates anything.
    assert!(removal_primary_group_allowed("alice", None, Some("alice")).is_ok());
    assert!(removal_primary_group_allowed("alice", Some("staff"), Some("staff")).is_ok());
    let error = removal_primary_group_allowed("alice", None, Some("staff"))
        .expect_err("a live primary group the declaration does not name must be refused");
    assert!(error.contains("primary group"), "{error}");
    // An unresolved gid carries no name to contradict the declaration.
    assert!(removal_primary_group_allowed("alice", None, None).is_ok());
}

#[test]
fn group_names_resolve_by_gid() {
    let content = "root:x:0:\ndaemon:x:1:\nstaff:x:50:\n";
    assert_eq!(
        super::super::guards::group_name_of_gid_from(content, 50),
        Some("staff".to_string())
    );
    assert_eq!(
        super::super::guards::group_name_of_gid_from(content, 0),
        Some("root".to_string())
    );
    assert_eq!(
        super::super::guards::group_name_of_gid_from(content, 9999),
        None
    );
}

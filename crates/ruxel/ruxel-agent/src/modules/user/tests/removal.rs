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
    assert!(group_removal_allowed("app", Some(1500), 1500, None).is_ok());
    assert!(group_removal_allowed("app", Some(1000), 1500, None).is_err());
    assert!(group_removal_allowed("wheel", None, 0, None).is_err());
    // A group a live account holds as its primary group is the
    // account's to keep: `groupdel` refuses to delete one without
    // `--force`, which this removal does not pass.
    assert!(group_removal_allowed("app", Some(1500), 1500, Some("alice")).is_err());
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

#[test]
fn a_removal_resolves_against_the_plan_s_account_state() {
    // An earlier task of the plan creates an initially missing account
    // and a later removal takes it away: check mode runs no `useradd`,
    // so the passwd file does not list the account — but the removal
    // must report itself, not the no-op of a name the unchanged file
    // never had: apply runs `userdel` on the account the plan made.
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let name = format!("ruxel-planned-removal-{}", std::process::id());
    run(&json!({"name": name, "create_home": false}), &ctx)
        .expect("the earlier creation must plan");
    let removal = run(&json!({"name": name, "state": "absent", "uid": 1500}), &ctx)
        .expect("the removal of the plan's own account must plan");
    assert_eq!(
        removal.get("changed"),
        Some(&json!(true)),
        "the planned account is a removal to report: {removal}"
    );
    // The same for a group the plan created.
    let group_name = format!("ruxel-planned-group-removal-{}", std::process::id());
    super::super::group(&json!({"name": group_name}), &ctx)
        .expect("the earlier group creation must plan");
    let removed = super::super::group(&json!({"name": group_name, "state": "absent"}), &ctx)
        .expect("the removal of the plan's own group must plan");
    assert_eq!(
        removed.get("changed"),
        Some(&json!(true)),
        "the planned group is a removal to report: {removed}"
    );
}

#[test]
fn a_removal_verifies_the_uid_against_the_planned_number() {
    // An earlier task renumbers an existing account from its live uid
    // to a planned one and a later removal declares a number: check
    // mode runs no `usermod`, so the passwd file still holds the stale
    // uid — verifying the declaration against it would refuse a
    // removal the apply performs after the renumbering.
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (name, live, gid) = passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid > 0)).then(|| {
                (
                    f[0].to_string(),
                    f[2].parse::<u64>().unwrap(),
                    f[3].parse::<u64>().unwrap(),
                )
            })
        })
        .expect("a non-root account on the host");
    let primary = super::super::guards::group_name_of_gid_from(&groups, gid);
    let planned = u32::try_from(live + 100_000).expect("the planned uid fits");
    run(&json!({"name": name, "uid": planned}), &ctx).expect("the earlier renumbering must plan");
    // The stale live uid must not stand in for the planned one: a
    // declaration naming it contradicts what the apply verifies
    // against and is refused.
    let error = run(
        &json!({"name": name, "state": "absent", "uid": live, "group": primary}),
        &ctx,
    )
    .expect_err("a declaration naming the stale uid must be refused");
    assert!(error.contains("refusing to remove"), "{error}");
    // The planned number is the one the removal's declaration is
    // verified against: the apply runs `usermod -u` first and `userdel`
    // then sees the renumbered entry.
    let removal = run(
        &json!({"name": name, "state": "absent", "uid": planned, "group": primary}),
        &ctx,
    )
    .expect("the removal must verify its declaration against the planned uid");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
}

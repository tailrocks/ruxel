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
    // Thread 336: the plan mechanics run over a pinned Debian-style
    // policy, so the removal is provable without the host's own
    // `/etc/login.defs`.
    let _policy = super::super::guards::pin_debian_policy();
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

fn apply_context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

/// A uid no live account holds: the plan's own, fitting the request
/// the creation records.
fn free_uid() -> u64 {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    (60000u64..61000)
        .find(|uid| {
            !passwd.lines().any(|line| {
                line.split(':')
                    .nth(2)
                    .is_some_and(|held| held == uid.to_string())
            })
        })
        .expect("a free uid on the host")
}

/// A live group nothing holds: no member of it, no account that holds
/// it as its primary group, and no account of its own name — the
/// removal's private-group scan finds it empty, while the group file
/// still lists it after a faked `userdel`.
fn empty_live_group() -> Option<(String, u64)> {
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    let accounts: Vec<&str> = passwd
        .lines()
        .filter_map(|line| line.split(':').next())
        .collect();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || f[2].parse::<u64>().is_err() {
            continue;
        }
        if f[2] == "0" || !f[3].is_empty() || accounts.contains(&f[0]) {
            continue;
        }
        let gid = f[2].parse::<u64>().unwrap_or(0);
        if super::super::guards::primary_group_holder_from(&passwd, gid, f[0]).is_none() {
            return Some((f[0].to_string(), gid));
        }
    }
    None
}

#[test]
fn apply_reinspects_the_group_file_after_userdel() {
    // On a host that keeps an account's empty same-name group
    // (`USERGROUPS_ENAB=no`), `userdel` leaves the group behind: the
    // apply re-inspects the group database after the removal really ran
    // and records no removal for a group that survived — a later
    // `Group`-present step resolves the live entry instead of attempting
    // `groupadd` on a group that exists. The host double is the live
    // group database (read-only) with `userdel` faked: the group the
    // fake deletion never touched is still listed.
    // Thread 336: the plan mechanics run over a pinned Debian-style
    // policy, so the removal is provable without the host's own
    // `/etc/login.defs`.
    let _policy = super::super::guards::pin_debian_policy();
    let (name, _gid) = empty_live_group().expect("an empty live group on the host");
    assert!(
        passwd_entry(&name).unwrap().is_none(),
        "no same-name account may exist on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let planned = free_uid();
    run(
        &json!({"name": name, "uid": planned, "create_home": false}),
        &check,
    )
    .expect("the creation must plan");
    // The fake absorbs `userdel`: the group file still lists the group,
    // the way a host that keeps empty same-name groups leaves it.
    super::super::test_hook::arm(vec![Ok(())]);
    let removal = run(
        &json!({"name": name, "state": "absent", "uid": planned}),
        &apply_context(),
    )
    .expect("the removal must run");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
    let calls = super::super::test_hook::take_calls();
    assert_eq!(calls.len(), 1, "userdel ran once: {calls:?}");
    assert_eq!(calls[0].0, "userdel");
    assert_eq!(
        calls[0].1,
        vec![name.clone()],
        "the removal takes the account without `-r`: {calls:?}"
    );
    assert!(
        !super::super::entries::group_removed(&name),
        "the surviving group is not recorded as removed"
    );
    // The later declaration resolves the live entry: no `groupadd`
    // runs, and the step reports no change — the check-mode prediction
    // with the tombstone would have attempted the creation instead.
    super::super::test_hook::arm(vec![]);
    let again = super::super::group(&json!({"name": name}), &apply_context())
        .expect("the redeclaration must resolve");
    assert_eq!(again.get("changed"), Some(&json!(false)), "{again}");
    assert!(
        super::super::test_hook::take_calls().is_empty(),
        "groupadd never ran: the entry was compared, not created"
    );
}

#[test]
fn apply_records_the_removal_where_the_group_is_gone() {
    // The mirror: the account's same-name group is not on the host, so
    // the `userdel` the fake absorbs takes the last trace of it — the
    // apply records the removal the way check mode predicts it.
    // Thread 336: the plan mechanics run over a pinned Debian-style
    // policy, so the removal is provable without the host's own
    // `/etc/login.defs`.
    let _policy = super::super::guards::pin_debian_policy();
    let name = format!("ruxel-gone-group-{}", std::process::id());
    assert!(
        group_entry(&name).unwrap().is_none(),
        "the group must not exist on the host"
    );
    assert!(
        passwd_entry(&name).unwrap().is_none(),
        "the account must not exist on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let planned = free_uid();
    run(
        &json!({"name": name, "uid": planned, "create_home": false}),
        &check,
    )
    .expect("the creation must plan");
    super::super::test_hook::arm(vec![Ok(())]);
    let removal = run(
        &json!({"name": name, "state": "absent", "uid": planned}),
        &apply_context(),
    )
    .expect("the removal must run");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
    assert_eq!(
        super::super::test_hook::take_calls().len(),
        1,
        "userdel ran once"
    );
    assert!(
        super::super::entries::group_removed(&name),
        "the group the removal took is recorded as removed"
    );
}

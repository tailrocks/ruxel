//! The account changes a check-mode plan makes without the live group
//! an earlier same-stage step has not created yet.

use super::super::*;

/// The check-mode context every plan in this file runs under: check
/// mode runs nothing, so the plan's own record is what the steps
/// resolve against.
fn check_context() -> ExecContext {
    ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

/// A host account no system keeps: a non-root account the plan may
/// renumber and remove.
fn host_account() -> (String, u64, u64) {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    passwd
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
        .expect("a non-root account on the host")
}

/// A group of the host no account holds as its primary group: the
/// removal of such a group is the host's to allow.
fn host_group() -> (String, u64) {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3
                && f[2].parse::<u64>().is_ok_and(|gid| gid > 0)
                && super::super::guards::primary_group_holder_from(
                    &passwd,
                    f[2].parse::<u64>().unwrap(),
                    f[0],
                )
                .is_none())
            .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
        })
        .expect("a group no account holds as its primary group")
}

#[test]
fn a_recreated_account_keeps_the_pending_ownership() {
    // The plan removes an account and an intervening task creates it
    // again without a number: the apply takes the account away and
    // creates it anew, so the allocator draws the number. The stale
    // live entry must not stand in for it — a file the plan owns with
    // that account would be reported unchanged while the apply chowns
    // it to the number the allocator drew. The requested ownership is
    // PENDING, like any other unnumbered identity.
    // Thread 336: the plan mechanics run over a pinned Debian-style
    // policy, so the removal is provable without the host's own
    // `/etc/login.defs`.
    let _policy = super::super::guards::pin_debian_policy();
    let check = check_context();
    let (name, live, gid) = host_account();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let primary = super::super::guards::group_name_of_gid_from(&groups, gid);
    let planned = live + 100_000;
    // The plan renumbers the account first: the removal verifies its
    // declaration against the planned number, which is no system
    // account's number.
    run(&json!({"name": name, "uid": planned}), &check).expect("the renumbering must plan");
    run(
        &json!({"name": name, "state": "absent", "uid": planned, "group": primary}),
        &check,
    )
    .expect("the removal must plan");
    // The intervening task creates the account again, without a number —
    // declaring the group explicitly, since a shared same-name group
    // is refused to an implicit adoption.
    run(
        &json!({"name": name, "group": primary, "create_home": false}),
        &check,
    )
    .expect("the creation must plan");
    let owned = crate::modules::resolve_ownership(
        json!({"owner": name}).as_object().expect("params object"),
        true,
    )
    .expect("the recreated account must resolve");
    assert!(
        owned.uid.is_none() && owned.uid_pending,
        "the recreated account holds no number yet: uid {:?} pending {}",
        owned.uid,
        owned.uid_pending
    );
}

#[test]
fn a_recreated_group_keeps_the_pending_ownership() {
    // The group counterpart: the plan removes a group and an
    // intervening task creates it again without a number, so the
    // allocator draws the gid and the stale live entry must not stand
    // in for it.
    // Thread 336: the plan mechanics run over a pinned Debian-style
    // policy, so the removal is provable without the host's own
    // `/etc/login.defs`.
    let _policy = super::super::guards::pin_debian_policy();
    let check = check_context();
    let (name, live) = host_group();
    let planned = live + 100_000;
    // The plan renumbers the group first: the removal verifies its
    // declaration against the planned number, which is no system
    // group's number.
    group(&json!({"name": name, "gid": planned}), &check).expect("the renumbering must plan");
    group(
        &json!({"name": name, "state": "absent", "gid": planned}),
        &check,
    )
    .expect("the removal must plan");
    group(&json!({"name": name}), &check).expect("the creation must plan");
    let owned = crate::modules::resolve_ownership(
        json!({"group": name}).as_object().expect("params object"),
        true,
    )
    .expect("the recreated group must resolve");
    assert!(
        owned.gid.is_none() && owned.gid_pending,
        "the recreated group holds no number yet: gid {:?} pending {}",
        owned.gid,
        owned.gid_pending
    );
}

#[test]
fn a_removal_verifies_the_declared_home_against_the_planned_account() {
    // The plan creates an account and a later stage removes it: check
    // mode runs no `useradd`, so the passwd file does not list the
    // account — but the removal verifies its declaration against the
    // account the plan made. `userdel -r` deletes that account's home,
    // so a declared home that is not it is refused before the utility
    // runs, exactly as it is for an account the host holds.
    let check = check_context();
    let name = format!("ruxel-planned-home-{}", std::process::id());
    run(
        &json!({"name": name, "home": "/srv/alice", "create_home": false}),
        &check,
    )
    .expect("the earlier creation must plan");
    let error = run(
        &json!({"name": name, "state": "absent", "remove": true, "home": "/srv/other"}),
        &check,
    )
    .expect_err("a recursive removal must verify the declared home against the plan's account");
    assert!(error.contains("userdel -r would delete"), "{error}");
    // The declared home that is the account's own home is accepted.
    let removal = run(
        &json!({"name": name, "state": "absent", "remove": true, "home": "/srv/alice"}),
        &check,
    )
    .expect("the declared home that is the account's own home must be accepted");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
}

#[test]
fn a_removal_verifies_the_declared_primary_group_against_the_planned_account() {
    // The primary-group counterpart: the plan creates an account whose
    // primary group is another one, and a later stage removes it
    // without naming the group. The private-group default of the
    // removal names the account's own group, which the account does
    // not hold — the removal refuses and asks for the real group, as
    // it does for an account the host holds.
    let check = check_context();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let primary = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid > 0)).then(|| f[0].to_string())
        })
        .expect("a group on the host");
    let name = format!("ruxel-planned-primary-{}", std::process::id());
    run(
        &json!({"name": name, "group": primary, "create_home": false}),
        &check,
    )
    .expect("the earlier creation must plan");
    let error = run(&json!({"name": name, "state": "absent"}), &check)
        .expect_err("a removal must verify the declared primary group against the plan's account");
    assert!(error.contains("primary group"), "{error}");
}

#[test]
fn a_removal_keeps_a_planned_same_name_group_the_account_does_not_hold() {
    // The plan creates the group `alice` and removes the account
    // `alice`, whose primary group is another one: `userdel` deletes
    // the account's empty same-name group only where that group is the
    // account's primary group, so the group the plan created survives
    // the removal — and the plan's record of it must survive it too,
    // or whatever resolves the group next fails in check mode where
    // the apply succeeds.
    let check = check_context();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let primary = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid > 0)).then(|| f[0].to_string())
        })
        .expect("a group on the host");
    let name = format!("ruxel-kept-group-{}", std::process::id());
    group(&json!({"name": name}), &check).expect("the earlier group step must plan");
    run(
        &json!({"name": name, "group": primary, "create_home": false}),
        &check,
    )
    .expect("the account step must plan");
    run(
        &json!({"name": name, "state": "absent", "group": primary}),
        &check,
    )
    .expect("the removal must plan");
    let owned = crate::modules::resolve_ownership(
        json!({"group": name}).as_object().expect("params object"),
        true,
    )
    .expect("the group's plan must survive the removal of the account");
    assert!(
        owned.gid.is_none() && owned.gid_pending,
        "the planned group survives the removal: gid {:?} pending {}",
        owned.gid,
        owned.gid_pending
    );
}

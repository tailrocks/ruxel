//! The identities a plan recreates or renumbers: the number the live
//! database holds after the apply, and the cache a task that names the
//! identity must not be served from.

use super::super::*;
use serde_json::json;

/// A host account no system keeps: a non-root account the plan may
/// renumber, remove and create anew.
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

#[test]
fn the_apply_resolves_a_recreated_account_from_the_live_database() {
    // The plan removes an account and an intervening task creates it
    // again without a number: the apply takes the account away and
    // creates it anew, so the allocator draws the number and the live
    // database holds it by the time the task that names the account
    // runs. The apply must resolve that number — a pending ownership
    // reports a change and chowns nothing, and the file keeps the
    // number of whoever claimed the freed one.
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
    run(&json!({"name": name, "create_home": false}), &check).expect("the creation must plan");
    let owned = crate::modules::resolve_ownership(
        json!({"owner": name}).as_object().expect("params object"),
        false,
    )
    .expect("the recreated account must resolve");
    assert_eq!(
        owned.uid,
        Some(u32::try_from(live).expect("the uid fits")),
        "the apply resolves the number the live database holds"
    );
    assert!(
        !owned.uid_pending,
        "the apply resolves the number: it is not pending"
    );
}

#[test]
fn a_renumbered_identity_makes_the_ownership_cache_stale() {
    // `usermod -u` renumbers the account but leaves an arbitrary file
    // at the old number, and the cached probe of a file weighs the
    // file's numbers alone: the task would be served from the cache
    // and skipped, and the file would keep the number the plan gave
    // the account. The plan's own record of the moved number is what
    // tells the cache is stale.
    let check = check_context();
    let (name, live, _) = host_account();
    let planned = live + 100_000;
    run(&json!({"name": name, "uid": planned}), &check).expect("the renumbering must plan");
    assert!(
        crate::modules::ownership_cache_stale("file", &json!({"path": "/srv/data", "owner": name})),
        "the file task names the account the plan renumbered"
    );
    assert!(
        !crate::modules::ownership_cache_stale(
            "file",
            &json!({"path": "/srv/data", "owner": "root"})
        ),
        "another owner is not the account the plan renumbered"
    );
    assert!(
        !crate::modules::ownership_cache_stale(
            "sysctl",
            &json!({"name": "net.test", "owner": name})
        ),
        "a task that owns nothing is not ownership-sensitive"
    );
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

/// A gid no live group holds, for the same reason.
fn free_gid() -> u64 {
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    (60000u64..61000)
        .find(|gid| {
            !groups.lines().any(|line| {
                line.split(':')
                    .nth(2)
                    .is_some_and(|held| held == gid.to_string())
            })
        })
        .expect("a free gid on the host")
}

#[test]
fn every_creation_marks_its_number_changed() {
    // Every creation draws a number no probe has seen yet — the
    // allocator's where the step states none, the declared one where
    // it does: an account the host deleted outside the plan and
    // creates again keeps no number, so a cached file the account
    // owned must not pass its old numeric probe and stay with whoever
    // claimed the freed number. The plan's record of the moved number
    // is what tells the cache it is stale.
    let check = check_context();
    let fresh = format!("ruxel-allocator-user-{}", std::process::id());
    assert!(
        passwd_entry(&fresh).unwrap().is_none(),
        "the account must not exist on the host"
    );
    run(&json!({"name": fresh, "create_home": false}), &check)
        .expect("the unnumbered creation must plan");
    assert!(
        user_number_changed(&fresh),
        "the allocator-drawn number is identity-changing"
    );
    assert!(
        crate::modules::ownership_cache_stale(
            "file",
            &json!({"path": "/srv/data", "owner": fresh})
        ),
        "a cached file the recreated account owns resolves stale"
    );
    // A creation that states its number is identity-changing too: the
    // number is one no probe has seen, and a file the deleted account
    // owned still holds the old one.
    let numbered = format!("ruxel-numbered-user-{}", std::process::id());
    run(
        &json!({"name": numbered, "uid": free_uid(), "create_home": false}),
        &check,
    )
    .expect("the numbered creation must plan");
    assert!(
        user_number_changed(&numbered),
        "a stated number is identity-changing too"
    );
    assert!(
        crate::modules::ownership_cache_stale(
            "file",
            &json!({"path": "/srv/data", "owner": numbered})
        ),
        "a cached file the numbered recreation owns resolves stale"
    );
}

#[test]
fn every_group_creation_marks_its_number_changed() {
    // The group counterpart: every creation draws a number no probe
    // has seen yet — the allocator's where the step states none, the
    // declared one where it does — so the plan's record tells the
    // cache it is stale either way.
    let check = check_context();
    let fresh = format!("ruxel-allocator-group-{}", std::process::id());
    assert!(
        group_entry(&fresh).unwrap().is_none(),
        "the group must not exist on the host"
    );
    super::super::group(&json!({"name": fresh}), &check)
        .expect("the unnumbered creation must plan");
    assert!(
        group_number_changed(&fresh),
        "the allocator-drawn number is identity-changing"
    );
    let numbered = format!("ruxel-numbered-group-{}", std::process::id());
    super::super::group(&json!({"name": numbered, "gid": free_gid()}), &check)
        .expect("the numbered creation must plan");
    assert!(
        group_number_changed(&numbered),
        "a stated number is identity-changing too"
    );
}

#[test]
fn a_numbered_recreation_keeps_its_planned_uid_at_removal() {
    // The plan removes an account, recreates it with an explicit uid,
    // and removes it again with a mismatched declared uid: the
    // recreation's number is the guard's to verify against — both check
    // and apply delete the recreated entry — so the mismatch is refused
    // instead of skipping the guards the way an unnumbered recreation
    // (whose number the allocator draws) must.
    let check = check_context();
    let name = format!("ruxel-recreated-uid-{}", std::process::id());
    assert!(
        passwd_entry(&name).unwrap().is_none(),
        "the account must not exist on the host"
    );
    let planned = free_uid();
    run(
        &json!({"name": name, "uid": planned, "create_home": false}),
        &check,
    )
    .expect("the creation must plan");
    run(
        &json!({"name": name, "state": "absent", "uid": planned}),
        &check,
    )
    .expect("the removal must plan");
    run(
        &json!({"name": name, "uid": planned, "create_home": false}),
        &check,
    )
    .expect("the recreation must plan");
    // The stale live number (here: no live number at all) must not
    // stand in for the planned one: the declaration names another
    // number and is refused.
    let error = run(
        &json!({"name": name, "state": "absent", "uid": planned + 1}),
        &check,
    )
    .expect_err("a declaration naming another number must be refused");
    assert!(error.contains("not the declared"), "{error}");
    // The matching declaration is the removal the apply performs.
    let removal = run(
        &json!({"name": name, "state": "absent", "uid": planned}),
        &check,
    )
    .expect("the matching removal must plan");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
}

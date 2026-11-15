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

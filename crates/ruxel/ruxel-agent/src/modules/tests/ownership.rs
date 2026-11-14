//! The file ownership resolution against the plan's own identities:
//! the requested number of an earlier `User` or `Group` task is what
//! the apply ends up with, not the stale live entry check mode leaves
//! behind.

use serde_json::json;

use super::super::ExecContext;

#[test]
fn the_plan_s_requested_uid_wins_over_the_stale_live_entry() {
    // An earlier `User` task renumbers an existing account from its
    // live uid to a planned one; check mode runs no `usermod`, so the
    // passwd file still holds the stale number and the resolution must
    // answer the number the apply ends up with — the file's chown is
    // planned against it, or the apply chowns it while check mode
    // reported it unchanged.
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let (name, live) = passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid > 0))
                .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
        })
        .expect("a non-root account on the host");
    let planned = u32::try_from(live + 100_000).expect("the planned uid fits");
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    super::super::user::run(&json!({"name": name, "uid": planned}), &check)
        .expect("the earlier account step must plan");
    let owned =
        super::resolve_ownership(json!({"owner": name}).as_object().expect("params object"))
            .expect("the plan's own renumbered account must resolve");
    assert_eq!(
        owned.uid,
        Some(planned),
        "the planned uid must win over the stale live {live}"
    );
}

#[test]
fn the_plan_s_requested_gid_wins_over_the_stale_live_entry() {
    // The gid counterpart: an earlier `Group` task renumbers an
    // existing group and check mode runs no `groupmod`.
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (name, live) = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid > 0))
                .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
        })
        .expect("a non-root group on the host");
    let planned = u32::try_from(live + 100_000).expect("the planned gid fits");
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    super::super::user::group(&json!({"name": name, "gid": planned}), &check)
        .expect("the earlier group step must plan");
    let owned =
        super::resolve_ownership(json!({"group": name}).as_object().expect("params object"))
            .expect("the plan's own renumbered group must resolve");
    assert_eq!(
        owned.gid,
        Some(planned),
        "the planned gid must win over the stale live {live}"
    );
}

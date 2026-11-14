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

#[test]
fn an_unnumbered_planned_identity_keeps_the_pending_ownership() {
    // An earlier same-stage step creates a missing account (and group)
    // without an explicit id, and a later `File` names it as the owner
    // of an existing, otherwise-correct path: check mode runs no
    // `useradd`, so the names resolve to no id before the apply
    // allocates one — but the apply creates the accounts, resolves the
    // allocated ids and chowns the file, so the requested ownership is
    // a PENDING change. Reporting it as "no owner constraint" lets the
    // check claim the file unchanged while the apply chowns it.
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let owner = format!("ruxel-pending-owner-{}", std::process::id());
    let group = format!("ruxel-pending-group-{}", std::process::id());
    super::super::user::run(&json!({"name": owner, "create_home": false}), &check)
        .expect("the earlier account step must plan");
    super::super::user::group(&json!({"name": group}), &check)
        .expect("the earlier group step must plan");
    let obj = json!({"owner": owner, "group": group})
        .as_object()
        .expect("params object")
        .clone();
    let ownership = super::resolve_ownership(&obj).expect("the planned identities must resolve");
    assert!(ownership.uid.is_none() && ownership.uid_pending);
    assert!(ownership.gid.is_none() && ownership.gid_pending);
    // The file is otherwise correct — its mode is not part of this
    // check — but the pending ids still report the ownership change
    // the apply makes.
    let path = std::env::temp_dir().join(format!("ruxel-pending-owned-{}", std::process::id()));
    std::fs::write(&path, b"").unwrap();
    let mut changed = false;
    super::super::attrs::apply_attrs_with(&path, &obj, &ownership, &mut changed, true)
        .expect("the pending ownership must plan");
    assert!(
        changed,
        "the pending ownership must be reported as a change"
    );
    std::fs::remove_file(&path).unwrap();
}

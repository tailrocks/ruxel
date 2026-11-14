//! The account changes a check-mode plan makes without the live group
//! an earlier same-stage step has not created yet.

use super::super::*;

#[test]
fn check_mode_plans_a_primary_group_that_does_not_exist_yet() {
    // An earlier same-stage `Group` step creates the group, and check
    // mode runs no `groupadd`: the live lookup that decides the
    // account's primary-group change fails on a clean host although
    // apply — which runs that step first — resolves it, so a dry run of
    // a workflow that succeeds when applied must not report failure.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| f[0].to_string())
        })
        .expect("a non-root account on the host");
    let group = format!("ruxel-planned-group-{}", std::process::id());
    assert!(
        group_entry(&group).unwrap().is_none(),
        "the planned group must not exist on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The earlier same-stage `Group` step of the plan: check mode runs
    // no `groupadd`, so the plan's record of that step is what the
    // account's plan resolves the new group against.
    let provided = super::super::group(&json!({"name": group}), &check)
        .expect("the earlier group step must plan");
    assert_eq!(provided.get("changed"), Some(&json!(true)), "{provided}");
    let result = run(&json!({"name": name, "group": group}), &check)
        .expect("check mode must plan the account change without the new group's live gid");
    assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
    // Apply mode still requires the live group: by then the earlier
    // step has run, and a group that is still missing is a real error
    // reported before `usermod` touches anything.
    let apply = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let error = run(&json!({"name": name, "group": group}), &apply)
        .expect_err("apply must fail where the requested group is not on the host");
    assert!(error.contains("not found"), "{error}");
}

#[test]
fn check_mode_refuses_a_missing_group_no_step_provides() {
    // An existing account whose primary group is missing on the host
    // and that no earlier task of the plan declares present: the
    // identical apply deterministically fails with `group ... not
    // found`, so the dry run must refuse the same way instead of
    // reporting a valid planned change.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| f[0].to_string())
        })
        .expect("a non-root account on the host");
    let group = format!("ruxel-unprovided-group-{}", std::process::id());
    assert!(
        group_entry(&group).unwrap().is_none(),
        "the group must not exist on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let error = run(&json!({"name": name, "group": group}), &check)
        .expect_err("check mode must refuse a missing group no task provides");
    assert!(error.contains("not found"), "{error}");
}

#[test]
fn a_file_owned_by_an_identity_the_plan_creates_resolves_in_check_mode() {
    // Check mode runs no `useradd` and no `groupadd`, so the passwd and
    // group files still lack the identities an earlier step of the plan
    // creates. A later file that names them resolves its ownership
    // through the plan's record — the requested ids where the steps
    // declared them — and plans the change instead of failing with
    // `user/group not found`: the identical apply creates the identity
    // first and succeeds.
    let group = format!("ruxel-planned-owner-group-{}", std::process::id());
    let account = format!("ruxel-planned-owner-user-{}", std::process::id());
    assert!(
        group_entry(&group).unwrap().is_none(),
        "the planned group must not exist on the host"
    );
    assert!(
        passwd_entry(&account).unwrap().is_none(),
        "the planned account must not exist on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    super::super::group(&json!({"name": group, "gid": 42424}), &check)
        .expect("the earlier group step must plan");
    run(
        &json!({"name": account, "uid": 42424, "group": group}),
        &check,
    )
    .expect("the earlier account step must plan");
    let dir = std::env::temp_dir().join(format!("ruxel-plan-owned-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("target");
    std::fs::write(&path, b"keep").unwrap();
    let owned = crate::modules::file::run(
        &json!({"path": path.to_str().unwrap(), "owner": account, "group": group}),
        &check,
    )
    .expect("a file owned by the plan's own identities must plan");
    assert_eq!(owned.get("changed"), Some(&json!(true)), "{owned}");
    // The requested ids are modeled: the pending change compares
    // against the numbers the apply will create.
    // A name no task of the plan provides is refused the way apply
    // refuses it: a dry run must not report a change that cannot
    // succeed.
    let error = crate::modules::file::run(
        &json!({"path": path.to_str().unwrap(), "owner": format!("ruxel-planned-owner-nobody-{}", std::process::id())}),
        &check,
    )
    .expect_err("an identity no step provides must be refused");
    assert!(error.contains("not found"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_mode_prefers_the_planned_gid_over_the_live_entry() {
    // An earlier check-mode `Group` task renumbers an existing group
    // from its live gid to a planned one (`groupmod -g`), and a later
    // existing `User` declares the group as its primary: check mode
    // runs nothing, so the live database still holds the stale number
    // — the primary-group change and the home repair must be planned
    // against the number the apply ends up with, or the dry run
    // reports no change while apply runs `groupmod`, `usermod -g` and
    // the repair.
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (name, live) = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2].parse::<u32>().is_ok_and(|gid| gid > 0))
                .then(|| (f[0].to_string(), f[2].parse::<u32>().unwrap()))
        })
        .expect("a non-root group on the host");
    let planned = live + 100_000;
    note_planned_group(&name, Some(planned));
    assert_eq!(
        planned_gid(&name, true).unwrap(),
        Some(planned),
        "the planned gid must win over the stale live {live} in check mode"
    );
    // Apply mode resolves from the host, whose `groupmod` has run by
    // then — the live entry is fresh there and stands.
    assert_eq!(planned_gid(&name, false).unwrap(), Some(live));
    forget_planned_group(&name);
}

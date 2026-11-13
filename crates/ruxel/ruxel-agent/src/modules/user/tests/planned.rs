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

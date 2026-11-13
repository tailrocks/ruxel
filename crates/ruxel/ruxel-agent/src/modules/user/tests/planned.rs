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

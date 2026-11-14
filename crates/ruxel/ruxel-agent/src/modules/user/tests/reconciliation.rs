//! Parameter checks and set-based supplementary-group reconciliation.

use super::super::*;
use super::strings;

fn desired_groups(current: &[String], want: &[String], append: bool) -> Vec<String> {
    let mut desired = if append { current.to_vec() } else { Vec::new() };
    desired.extend(want.iter().cloned());
    desired.sort();
    desired.dedup();
    desired
}

#[test]
fn groups_reject_scalars_and_non_string_entries() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    for groups in [json!("wheel,docker"), json!(["wheel", 2])] {
        assert!(
            run(
                &json!({"name":"ruxel-synthetic-user", "groups":groups}),
                &ctx
            )
            .is_err()
        );
        assert!(
            run(
                &json!({"name":"ruxel-synthetic-user", "state":"absent", "groups":groups}),
                &ctx
            )
            .is_err()
        );
    }
    // The named groups must resolve for the plan to report a creation,
    // so the names are the host's own: what this test is about is the
    // parameter's shape, not the group's existence.
    let host = std::fs::read_to_string("/etc/group").unwrap();
    let live: Vec<String> = host
        .lines()
        .filter_map(|line| line.split(':').next().map(str::to_string))
        .filter(|name| !name.is_empty() && !name.starts_with('#'))
        .filter(|name| group_entry(name).is_ok_and(|entry| entry.is_some()))
        .take(2)
        .collect();
    assert!(live.len() == 2, "the host must have two groups");
    assert!(run(&json!({"name":"ruxel-synthetic-user", "groups":live}), &ctx).is_ok());
}

#[test]
fn rejects_option_like_group_names_before_running_utilities() {
    for check_mode in [true, false] {
        let ctx = ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        for state in ["present", "absent"] {
            for name in ["--help", "-g"] {
                let error = group(&json!({"name":name, "state":state}), &ctx)
                    .expect_err("option-like group name must be rejected");
                assert!(error.contains("group") && error.contains("name"));
            }
        }
        if check_mode {
            assert!(group(&json!({"name":"ruxel-synthetic-group"}), &ctx).is_ok());
        }
    }
}

#[test]
fn rejects_option_like_user_names_before_running_utilities() {
    for check_mode in [true, false] {
        let ctx = ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        for state in ["present", "absent"] {
            for name in ["--help", "-u"] {
                let error = run(&json!({"name":name, "state":state}), &ctx)
                    .expect_err("option-like user name must be rejected");
                assert!(error.contains("user") && error.contains("name"));
            }
        }
        if check_mode {
            assert!(run(&json!({"name":"ruxel-synthetic-user"}), &ctx).is_ok());
        }
    }
}

#[test]
fn supplementary_group_reconciliation_is_set_based() {
    assert_eq!(
        desired_groups(&strings(&["a"]), &strings(&["b", "b"]), true),
        strings(&["a", "b"])
    );
    assert!(groups_need_update(&strings(&["a"]), &strings(&["b"]), true));
    assert!(!groups_need_update(
        &strings(&["a", "b"]),
        &strings(&["b", "a"]),
        false
    ));
    assert_eq!(
        desired_groups(&strings(&["a", "b"]), &strings(&["b"]), false),
        strings(&["b"])
    );
}

#[test]
fn an_existing_account_reconciles_its_implicit_primary_group() {
    // The step's implicit claim is the account's PRIMARY group too: the
    // schema models the same-name group as the primary group of an
    // account that names none, so an existing account left on another
    // group must be moved onto it (`usermod -g`) — the group's creation
    // alone leaves the account on `staff`, invalidating the lowering's
    // `primaryGroup` decisions and the declared account state
    // unapplied. An account already on its same-name group stays
    // unchanged.
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    // The host's own account whose primary group is not its same-name
    // group (the group missing, or carrying another gid): the move is
    // the change the step plans.
    let unreconciled = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7
                && f[2].parse::<u64>().is_ok_and(|uid| uid != 0)
                && !f[0].starts_with(['+', '#', '-']))
            .then(|| {
                (
                    f[0].to_string(),
                    f[3].parse::<u64>().expect("a group number"),
                )
            })
        })
        .find(|(account, gid)| {
            group_entry(account)
                .unwrap()
                .is_none_or(|(_, same_name_gid)| same_name_gid != *gid)
        })
        .map(|(account, _)| account);
    if let Some(name) = unreconciled {
        let result =
            run(&json!({"name": name}), &check).expect("the implicit primary-group move must plan");
        assert_eq!(
            result.get("changed"),
            Some(&json!(true)),
            "the account left off its same-name group is moved onto it: {result}"
        );
    }
    // An account already carrying its same-name group as its primary
    // one: the step reconciles nothing and reports no change.
    let reconciled = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7
                && f[2].parse::<u64>().is_ok_and(|uid| uid != 0)
                && !f[0].starts_with(['+', '#', '-']))
            .then(|| {
                (
                    f[0].to_string(),
                    f[3].parse::<u64>().expect("a group number"),
                )
            })
        })
        .find(|(account, gid)| {
            group_entry(account)
                .unwrap()
                .is_some_and(|(_, same_name_gid)| same_name_gid == *gid)
        })
        .map(|(account, _)| account)
        .expect("a live account on its same-name group");
    let result = run(&json!({"name": reconciled}), &check).expect("the step must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(false)),
        "an account already on its same-name group is unchanged: {result}"
    );
}

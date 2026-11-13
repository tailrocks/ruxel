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

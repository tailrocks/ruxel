use super::*;

#[test]
fn failed_update_invalidates_cached_packages_and_reports_possible_change() {
    let mut state = SystemState::with_test_packages();
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let result = run_with(
        &json!({"update_cache": true}),
        &ctx,
        &mut state,
        &mut |_, _| Ok((String::new(), 1, "failure".into())),
    )
    .unwrap();
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], true);
    assert!(state.test_packages_invalidated());
}

mod process;

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![("FIXTURE".into(), "value".into())],
        become_user: None,
    }
}

fn summary(changed: bool) -> String {
    format!(
        "0 upgraded, 0 newly installed, {} to remove and 0 not upgraded.\n",
        u8::from(changed)
    )
}

#[test]
fn upgrade_and_autoremove_both_run_and_aggregate_changes() -> Result<(), String> {
    for check in [false, true] {
        for upgrade_changed in [false, true] {
            for removed in [false, true] {
                let mut calls = Vec::new();
                let mut command = |args: &[&str], ctx: &ExecContext| {
                    assert_eq!(ctx.environment, [("FIXTURE".into(), "value".into())]);
                    calls.push(args.iter().map(ToString::to_string).collect::<Vec<_>>());
                    let changed = if args.last() == Some(&"dist-upgrade") {
                        upgrade_changed
                    } else {
                        removed
                    };
                    Ok((summary(changed), 0, String::new()))
                };
                let result = run_with(
                    &json!({"upgrade":"dist","autoremove":true}),
                    &context(check),
                    &mut SystemState::default(),
                    &mut command,
                )?;
                let expected = if check {
                    vec![vec!["-s", "dist-upgrade"], vec!["-s", "autoremove"]]
                } else {
                    vec![vec!["dist-upgrade"], vec!["autoremove"]]
                };
                assert_eq!(calls, expected);
                assert_eq!(result["changed"], upgrade_changed || removed);
                assert_eq!(result["failed"], false);
                assert_eq!(result["stdout"], summary(removed).trim_end());
            }
        }
    }
    Ok(())
}

#[test]
fn maintenance_failures_stop_execution_and_report_the_action() {
    for check in [false, true] {
        for fail_action in ["dist-upgrade", "autoremove"] {
            let mut calls = Vec::new();
            let mut command = |args: &[&str], _: &ExecContext| {
                let action = args.last().copied().unwrap_or_default();
                calls.push(action.to_string());
                Ok((
                    summary(false),
                    i32::from(action == fail_action),
                    "fixture refusal".into(),
                ))
            };
            let error = run_with(
                &json!({"upgrade":"dist","autoremove":true}),
                &context(check),
                &mut SystemState::default(),
                &mut command,
            )
            .err()
            .unwrap_or_default();
            assert!(error.contains(fail_action) && error.contains("fixture refusal"));
            assert_eq!(
                calls,
                if fail_action == "dist-upgrade" {
                    vec!["dist-upgrade"]
                } else {
                    vec!["dist-upgrade", "autoremove"]
                }
            );
        }
    }
}

#[test]
fn standalone_autoremove_and_upgrade_keep_their_selected_actions() -> Result<(), String> {
    for (params, expected) in [
        (json!({"autoremove":true}), vec!["autoremove"]),
        (json!({"upgrade":"dist"}), vec!["dist-upgrade"]),
        (json!({}), vec![]),
    ] {
        let mut calls = Vec::new();
        let result = run_with(
            &params,
            &context(false),
            &mut SystemState::default(),
            &mut |args, _| {
                calls.extend(args.iter().map(ToString::to_string));
                Ok((summary(false), 0, String::new()))
            },
        )?;
        assert_eq!(calls, expected);
        assert_eq!(result["changed"], false);
    }
    Ok(())
}

#[test]
fn summary_parsing() {
    for (text, changed) in [
        (
            "0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.",
            false,
        ),
        (
            "12 upgraded, 1 newly installed, 0 to remove and 3 not upgraded.",
            true,
        ),
        (
            "0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded.",
            true,
        ),
        (
            "0 upgraded, 0 newly installed, 0 to remove and 7 not upgraded.",
            false,
        ),
    ] {
        assert_eq!(summary_changed(text), changed);
    }
}

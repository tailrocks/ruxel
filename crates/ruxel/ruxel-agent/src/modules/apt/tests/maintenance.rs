use super::*;

#[test]
fn upgrade_and_autoremove_both_run_and_aggregate_changes() -> Result<(), String> {
    for check in [false, true] {
        for upgrade_changed in [false, true] {
            for removed in [false, true] {
                let mut calls = Vec::new();
                let mut command = |args: &[&str], ctx: &ExecContext| {
                    assert_eq!(ctx.environment, [("FIXTURE".into(), "value".into())]);
                    calls.push(args.iter().map(ToString::to_string).collect::<Vec<_>>());
                    let changed = if args.contains(&"--auto-remove") {
                        // The combined simulation speaks for both parts.
                        upgrade_changed || removed
                    } else if args.last() == Some(&"dist-upgrade") {
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
                // In check and diff mode the two operations are simulated
                // as one transaction: a separate `-s autoremove` would
                // evaluate the pre-upgrade dependency graph while apply
                // evaluates the post-upgrade one.
                let expected = if check {
                    vec![vec!["-s", "--auto-remove", "dist-upgrade"]]
                } else {
                    vec![vec!["dist-upgrade"], vec!["autoremove"]]
                };
                assert_eq!(calls, expected);
                assert_eq!(result["changed"], upgrade_changed || removed);
                assert_eq!(result["failed"], false);
                let expected_stdout = if check {
                    summary(upgrade_changed || removed)
                } else {
                    summary(removed)
                };
                assert_eq!(result["stdout"], expected_stdout.trim_end());
            }
        }
    }
    Ok(())
}

/// A failed maintenance action stops the run and is reported in the
/// result: the change the run accumulated stays visible beside the
/// failure instead of being hidden behind an error.
#[test]
fn maintenance_failures_stop_execution_and_report_the_action() -> Result<(), String> {
    for check in [false, true] {
        for fail_action in ["dist-upgrade", "autoremove"] {
            if check && fail_action == "autoremove" {
                // Check and diff simulate the upgrade and the autoremove
                // as one transaction: there is no separate autoremove to
                // fail.
                continue;
            }
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
            let result = run_with(
                &json!({"upgrade":"dist","autoremove":true}),
                &context(check),
                &mut SystemState::default(),
                &mut command,
            )?;
            assert_eq!(result["failed"], true);
            assert_eq!(result["changed"], false);
            let message = result["msg"].as_str().unwrap_or_default();
            assert!(
                message.contains(fail_action) && message.contains("fixture refusal"),
                "{message}"
            );
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
    Ok(())
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

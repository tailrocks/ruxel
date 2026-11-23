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
                    if args == ["showhold"] {
                        return Ok((String::new(), 0, String::new()));
                    }
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
                    vec![
                        vec!["showhold"],
                        vec!["-s", "--auto-remove", "dist-upgrade"],
                    ]
                } else {
                    vec![vec!["showhold"], vec!["dist-upgrade"], vec!["autoremove"]]
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
                if action == "showhold" {
                    return Ok((String::new(), 0, String::new()));
                }
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
            // A failed apply-mode maintenance may have changed the host
            // before it stopped, so it conservatively reports the change;
            // the simulation changed nothing.
            assert_eq!(result["changed"], !check);
            let message = result["msg"].as_str().unwrap_or_default();
            assert!(
                message.contains(fail_action) && message.contains("fixture refusal"),
                "{message}"
            );
            assert_eq!(
                calls,
                if fail_action == "dist-upgrade" {
                    vec!["showhold", "dist-upgrade"]
                } else {
                    vec!["showhold", "dist-upgrade", "autoremove"]
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
        (json!({"upgrade":"dist"}), vec!["showhold", "dist-upgrade"]),
        (json!({}), vec![]),
    ] {
        let mut calls = Vec::new();
        let result = run_with(
            &params,
            &context(false),
            &mut SystemState::default(),
            &mut |args, _| {
                calls.extend(args.iter().map(ToString::to_string));
                if args == ["showhold"] {
                    return Ok((String::new(), 0, String::new()));
                }
                Ok((summary(false), 0, String::new()))
            },
        )?;
        assert_eq!(calls, expected);
        assert_eq!(result["changed"], false);
    }
    Ok(())
}

/// A run that merely configures an unpacked package or processes
/// pending triggers changes the host although every summary counter is
/// zero: apt-get reports the pending dpkg work on its own line —
/// `2 not fully installed or removed.` — and the watchers of the
/// maintenance step must see that change.
#[test]
fn pending_configuration_counts_as_a_maintenance_change() {
    let text = "0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n\
                2 not fully installed or removed.\n\
                Need to get 0 B of archives.\n";
    assert!(summary_changed(text), "{text:?}");
    // A counter of zero pending packages is no change.
    let clean = "0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n\
                 0 not fully installed or removed.\n";
    assert!(!summary_changed(clean), "{clean:?}");
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
        // Apt's format-30 summary: the same counters under another
        // format a host apt.conf can select.
        (
            "Summary:\n  Upgrading: 1, Installing: 0, Removing: 0, Not Upgrading: 0",
            true,
        ),
        (
            "Summary:\n  Upgrading: 0, Installing: 2, Removing: 0, Not Upgrading: 0",
            true,
        ),
        (
            "Summary:\n  Upgrading: 0, Installing: 0, Removing: 3, Not Upgrading: 0",
            true,
        ),
        (
            "Summary:\n  Upgrading: 0, Installing: 0, Removing: 0, Not Upgrading: 7",
            false,
        ),
    ] {
        assert_eq!(summary_changed(text), changed);
    }
}

/// A maintenance run can stop services and autoremove their units, so
/// the unit snapshot goes with the package snapshot: a later service
/// step must read the host again instead of trusting cached `active` or
/// `enabled` values the maintenance moved past. Check mode simulates
/// nothing and keeps both snapshots.
#[test]
fn a_maintenance_run_invalidates_the_unit_snapshot() -> Result<(), String> {
    for check in [false, true] {
        let mut state = SystemState::with_test_units();
        let result = run_with(
            &json!({"autoremove": true}),
            &context(check),
            &mut state,
            &mut |_, _| Ok((summary(false), 0, String::new())),
        )?;
        assert_eq!(result["failed"], false, "{check}");
        assert_eq!(state.test_units_invalidated(), !check, "{check}");
    }
    Ok(())
}

/// A hold-discovery failure must not hide the changes the run already
/// made: `apt-get update` completes and then `apt-mark showhold` fails,
/// and the result keeps the refresh's change beside the failure instead
/// of surfacing a bare error that `modules::execute` reports as
/// `changed: false`. Both discovery paths — the upgrade's pre-flight and
/// the hold step's read — carry the change.
#[test]
fn a_hold_discovery_failure_keeps_the_prior_change() -> Result<(), String> {
    for params in [
        json!({"update_cache": true, "upgrade": "dist"}),
        json!({"update_cache": true, "hold": true, "name": "pkg"}),
    ] {
        let mut command = |args: &[&str], _: &ExecContext| match args {
            ["update"] => Ok((summary(true), 0, String::new())),
            ["showhold"] => Err("fixture refusal".into()),
            other => Err(format!("unexpected apt call {other:?}")),
        };
        let result = run_with(
            &params,
            &context(false),
            &mut SystemState::default(),
            &mut command,
        )
        .map_err(|error| format!("{params}: the refresh was hidden behind {error:?}"))?;
        assert_eq!(result["failed"], true, "{params}");
        assert_eq!(
            result["changed"], true,
            "{params}: the completed refresh was hidden"
        );
        let message = result["msg"].as_str().unwrap_or_default();
        assert!(message.contains("refusal"), "{message}");
    }
    Ok(())
}

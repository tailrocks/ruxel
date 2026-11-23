//! Failures that stop a run after the host may already have changed: the
//! change the run accumulated stays in the report, so a completed
//! installation or upgrade is never hidden behind an error that
//! `modules::execute` turns into `changed: false`.

use super::*;
use std::sync::{Arc, Mutex};

fn status(packages: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for (name, version) in packages {
        write!(
            text,
            "Package: {name}\nStatus: install ok installed\nArchitecture: amd64\nVersion: {version}\n\n"
        )
        .expect("write to string");
    }
    text
}

/// A completed installation is not hidden when the hold after it fails:
/// `apt-mark hold` or its read-back can fail once apt-get succeeded, and
/// the result keeps the install's change alongside the failure.
#[test]
fn a_holding_failure_after_a_changing_install_reports_the_install_change() -> Result<(), String> {
    for fail in ["mark", "read-back"] {
        let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
        let mut state = SystemState::with_dpkg_status(database.clone());
        let mut showholds = 0;
        let mut command = |args: &[&str], _: &ExecContext| match args {
            ["showhold"] => {
                showholds += 1;
                let refused = fail == "read-back" && showholds > 1;
                Ok((
                    String::new(),
                    i32::from(refused),
                    "read-back refused".into(),
                ))
            }
            ["hold", ..] => Ok((
                String::new(),
                i32::from(fail == "mark"),
                "mark refused".into(),
            )),
            ["install", ..] => {
                *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
                Ok((summary(true), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        };
        let result = run_with(
            &json!({"name": "pkg=2", "hold": true}),
            &context(false),
            &mut state,
            &mut command,
        )
        .map_err(|error| format!("{fail}: the run hid the install behind {error:?}"))?;
        assert_eq!(result["changed"], true, "{fail}");
        assert_eq!(result["failed"], true, "{fail}");
        let message = result["msg"].as_str().unwrap_or_default();
        assert!(
            message.contains("apt-mark") && message.contains("refused"),
            "{fail}: {message}"
        );
    }
    Ok(())
}

/// A whole-system upgrade that changed the host is not hidden when the
/// autoremove after it fails: the result keeps the upgrade's change
/// alongside the failure. (Check and diff simulate the two operations as
/// one transaction, so the separate autoremove — and this failure — is
/// an apply-time shape.)
#[test]
fn an_autoremove_failure_after_a_changing_upgrade_reports_the_upgrade_change() -> Result<(), String>
{
    {
        let mut calls = Vec::new();
        let mut command = |args: &[&str], _: &ExecContext| {
            let action = args.last().copied().unwrap_or_default();
            calls.push(action.to_string());
            if action == "showhold" {
                return Ok((String::new(), 0, String::new()));
            }
            Ok((
                summary(action != "autoremove"),
                i32::from(action == "autoremove"),
                "autoremove refused".into(),
            ))
        };
        let result = run_with(
            &json!({"upgrade": "dist", "autoremove": true}),
            &context(false),
            &mut SystemState::default(),
            &mut command,
        )
        .map_err(|error| format!("{calls:?}: the run hid the upgrade behind {error:?}"))?;
        assert_eq!(result["changed"], true, "{calls:?}");
        assert_eq!(result["failed"], true, "{calls:?}");
        let message = result["msg"].as_str().unwrap_or_default();
        assert!(
            message.contains("autoremove") && message.contains("refused"),
            "{message}"
        );
    }
    Ok(())
}

/// A hold that took effect is not hidden when the read-back afterwards
/// fails: the package was installed and unheld, `apt-mark hold` marked it,
/// and the host changed. The result reports the change alongside the
/// failure — distinct from the installation case above, where the change
/// preceded the marks.
#[test]
fn a_read_back_failure_after_a_successful_hold_reports_the_change() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let mut showholds = 0;
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["showhold"] => {
            showholds += 1;
            Ok((
                String::new(),
                i32::from(showholds > 1),
                "read-back refused".into(),
            ))
        }
        ["hold", ..] => Ok((String::new(), 0, String::new())),
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let result = run_with(
        &json!({"name": "pkg", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )
    .map_err(|error| format!("the run hid the hold behind {error:?}"))?;
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], true);
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(
        message.contains("read back") && message.contains("refused"),
        "{message}"
    );
    Ok(())
}

/// A failed apply-mode maintenance may have mutated dpkg before it
/// stopped: the snapshot an earlier plan took would answer "present"
/// for the version the failed run moved away. The snapshot is dropped,
/// so the next plan reads dpkg again instead of the stale cache.
#[test]
fn a_failed_maintenance_invalidates_the_package_snapshot() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let request = PackageRequest::parse("pkg=1")?;
    // An earlier step's plan snapshots the host: the pin is satisfied.
    let before = state.plan_packages(std::slice::from_ref(&request), false, &[])?;
    assert!(before.missing.is_empty(), "{before:?}");
    // The failed run moved the package away before it stopped.
    let mut command = |args: &[&str], _: &ExecContext| {
        if args == ["showhold"] {
            return Ok((String::new(), 0, String::new()));
        }
        *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
        Ok((summary(false), 1, "upgrade refused".into()))
    };
    let result = run_with(
        &json!({"upgrade": "dist"}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], true);
    let after = state.plan_packages(std::slice::from_ref(&request), false, &[])?;
    assert!(
        !after.missing.is_empty(),
        "the plan answered present from a stale snapshot"
    );
    Ok(())
}

/// A hold restoration that fails beside a failing transaction is not
/// hidden behind the transaction's error: the run left the package
/// unintentionally unheld, and the report must name both failures and
/// keep that known change. The test this replaces expected a bare
/// `Err` — that sequence then surfaced through `modules::execute` as
/// `changed: false`, hiding the unheld package behind the error; it is
/// changed deliberately.
#[test]
fn a_failed_restoration_beside_a_failed_install_reports_the_known_change() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["showhold"] => Ok(("pkg\n".into(), 0, String::new())),
        ["unhold", ..] => Ok((String::new(), 0, String::new())),
        ["install", ..] => Ok((summary(false), 1, "install refused".into())),
        ["hold", ..] => Ok((String::new(), 1, "restore refused".into())),
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )
    .map_err(|error| format!("the run hid the failure behind {error:?}"))?;
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], true, "the unheld package was hidden");
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(message.contains("install refused"), "{message}");
    assert!(
        message.contains("restore refused"),
        "the restoration failure was dropped: {message}"
    );
    Ok(())
}

/// A system upgrade refuses a pre-existing hold instead of leaving the
/// held package behind: a plain `dist-upgrade` runs without
/// `--ignore-hold`, so the whole-system result would be false, and the
/// only option that moves a held package is the dangerous override
/// apt-get(8) documents. The run names the holds and the upgrade never
/// runs — the agent-side twin of the Lowering's refusal of a hold
/// declared beside an upgrade.
#[test]
fn a_pre_existing_hold_refuses_the_system_upgrade() -> Result<(), String> {
    for check in [false, true] {
        let mut upgraded = false;
        let mut command = |args: &[&str], _: &ExecContext| {
            if args == ["showhold"] {
                return Ok(("curl\n".into(), 0, String::new()));
            }
            if args.last() == Some(&"dist-upgrade") {
                upgraded = true;
            }
            Ok((summary(false), 0, String::new()))
        };
        let result = run_with(
            &json!({"upgrade": "dist"}),
            &context(check),
            &mut SystemState::default(),
            &mut command,
        )?;
        assert_eq!(result["failed"], true, "{check}");
        assert!(!upgraded, "{check}: the upgrade ran despite the hold");
        let message = result["msg"].as_str().unwrap_or_default();
        assert!(
            message.contains("curl") && message.contains("unhold"),
            "{message}"
        );
    }
    Ok(())
}

/// A hold transition whose `apt-mark unhold` partially fails restores
/// the holds it released before the failure is reported: apt-mark can
/// lift the valid selectors before a bad one exits nonzero, and the host
/// must not keep a silently dropped hold. The error names both the
/// failure and the restoration.
#[test]
fn a_partially_failing_unhold_restores_the_released_holds() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1"), ("gone", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let mut reheld: Vec<String> = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["showhold"] => Ok(("pkg\ngone\n".into(), 0, String::new())),
        ["unhold", ..] => Ok((String::new(), 1, "E: no package 'gone'".into())),
        ["hold", names @ ..] => {
            reheld.extend(names.iter().map(ToString::to_string));
            Ok((String::new(), 0, String::new()))
        }
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let result = run_with(
        &json!({"name": ["pkg=2", "gone=2"], "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(result["failed"], true);
    assert!(!reheld.is_empty(), "the released holds were not restored");
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(
        message.contains("apt-mark unhold failed") && message.contains("restored"),
        "{message}"
    );
    Ok(())
}

/// A failed held-pin install reports the known change: apt-get can move
/// one package and then stop in a maintainer script, so the transaction
/// may have changed the host even though it exited nonzero — and even
/// though the hold restoration succeeded. The partial-change signal
/// does not depend on the lost-hold flag alone.
#[test]
fn a_failed_held_install_reports_the_known_change() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["showhold"] => Ok(("pkg\n".into(), 0, String::new())),
        ["unhold" | "hold", ..] => Ok((String::new(), 0, String::new())),
        ["install", ..] => {
            *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
            Ok((summary(true), 1, "maintainer script refused".into()))
        }
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(result["failed"], true);
    assert_eq!(
        result["changed"], true,
        "the partial change the failed install left was hidden"
    );
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(
        message.contains("maintainer script refused") && message.contains("restored"),
        "{message}"
    );
    Ok(())
}

/// A hold command that partially applies reconciles the marks it
/// persisted: `apt-mark hold` can keep the valid selectors before a bad
/// one exits nonzero, and those marks are a known change the report must
/// carry — beside the failure — naming the holds the host really has,
/// instead of dropping them behind a bare error that claims `changed:
/// false`.
#[test]
fn a_partially_failing_hold_reconciles_the_marks_it_persisted() -> Result<(), String> {
    let database = Arc::new(Mutex::new(status(&[("pkg", "1"), ("gone", "1")])));
    let mut state = SystemState::with_dpkg_status(database.clone());
    let mut showholds = 0;
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["showhold"] => {
            showholds += 1;
            // The read-back names only the mark that persisted.
            Ok((
                if showholds > 1 { "pkg\n" } else { "" }.into(),
                0,
                String::new(),
            ))
        }
        ["hold", ..] => Ok((String::new(), 1, "E: no package 'gone'".into())),
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let result = run_with(
        &json!({"name": ["pkg", "gone"], "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(result["failed"], true);
    assert_eq!(
        result["changed"], true,
        "the marks the partial hold persisted were hidden"
    );
    assert_eq!(result["holds"], json!(["pkg"]));
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(
        message.contains("apt-mark hold failed") && message.contains("gone"),
        "{message}"
    );
    Ok(())
}

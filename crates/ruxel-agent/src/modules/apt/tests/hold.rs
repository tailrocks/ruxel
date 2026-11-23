use super::*;
use std::sync::{Arc, Mutex};

fn status(packages: &[(&str, &str)]) -> String {
    let packages: Vec<(&str, &str, &str)> = packages
        .iter()
        .map(|(name, version)| (*name, *version, "amd64"))
        .collect();
    status_with_arch(&packages)
}

fn status_with_arch(packages: &[(&str, &str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for (name, version, arch) in packages {
        write!(
            text,
            "Package: {name}\nStatus: install ok installed\nArchitecture: {arch}\nVersion: {version}\n\n"
        )
        .expect("write to string");
    }
    text
}

fn host(packages: &[(&str, &str)]) -> (SystemState, Arc<Mutex<String>>) {
    let database = Arc::new(Mutex::new(status(packages)));
    (SystemState::with_dpkg_status(database.clone()), database)
}

fn host_with_arch(packages: &[(&str, &str, &str)]) -> (SystemState, Arc<Mutex<String>>) {
    let database = Arc::new(Mutex::new(status_with_arch(packages)));
    (SystemState::with_dpkg_status(database.clone()), database)
}

fn calls_of(calls: &mut Vec<Vec<String>>, args: &[&str]) {
    calls.push(args.iter().map(ToString::to_string).collect::<Vec<_>>());
}

#[test]
fn hold_installs_then_marks_held_and_reports_holds() -> Result<(), String> {
    let (mut state, database) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let mut held = false;
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok((
                if held { "pkg\n".into() } else { String::new() },
                0,
                String::new(),
            )),
            ["hold", ..] => {
                held = true;
                Ok((String::new(), 0, String::new()))
            }
            ["install", ..] => {
                *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
                Ok((summary(true), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(
        calls,
        vec![
            vec!["showhold"],
            vec!["install", "pkg=2"],
            vec!["hold", "pkg"],
            vec!["showhold"]
        ]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg"]));
    Ok(())
}

/// A request that names an architecture is held with that architecture:
/// `apt-mark hold pkg:arm64`, not `apt-mark hold pkg`.
#[test]
fn hold_keeps_the_architecture_of_the_request() -> Result<(), String> {
    let (mut state, _) = host_with_arch(&[("pkg", "1", "arm64")]);
    let mut calls = Vec::new();
    let mut held = false;
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok((
                if held {
                    "pkg:arm64\n".into()
                } else {
                    String::new()
                },
                0,
                String::new(),
            )),
            ["hold", ..] => {
                held = true;
                Ok((String::new(), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg:arm64", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(
        calls,
        vec![
            vec!["showhold"],
            vec!["hold", "pkg:arm64"],
            vec!["showhold"]
        ]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg:arm64"]));
    Ok(())
}

/// A hold on the unqualified `pkg` is not a hold on `pkg:arm64`: the two
/// name different packages, so the request is held again.
#[test]
fn hold_does_not_confuse_a_package_with_its_architecture() -> Result<(), String> {
    let (mut state, _) = host_with_arch(&[("pkg", "1", "arm64")]);
    let mut calls = Vec::new();
    let mut held = false;
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok((
                if held {
                    "pkg\npkg:arm64\n".into()
                } else {
                    "pkg\n".into()
                },
                0,
                String::new(),
            )),
            ["hold", ..] => {
                held = true;
                Ok((String::new(), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg:arm64", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(
        calls,
        vec![
            vec!["showhold"],
            vec!["hold", "pkg:arm64"],
            vec!["showhold"]
        ]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg", "pkg:arm64"]));
    Ok(())
}

/// A hold the host reports must name a package the agent can name: a line
/// that is not a package selector is refused, not accepted as a hold.
#[test]
fn hold_refuses_a_malformed_showhold_line() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1")]);
    // The refusal is the failed report, not a bare error: the report
    // carries the change any earlier command already made beside it.
    let result = run_with(
        &json!({"name": "pkg", "hold": true}),
        &context(false),
        &mut state,
        &mut |args, _| match args {
            ["showhold"] => Ok(("pkg\nlib*\n".into(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        },
    )
    .map_err(|error| format!("the refusal hid the malformed line behind {error:?}"))?;
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], false);
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(message.contains("invalid package selector"), "{message}");
    Ok(())
}

/// The marks are only as good as the state the host reports: a package the
/// host does not list after the hold is not held, and the report says so.
#[test]
fn hold_reports_the_failure_when_the_read_back_does_not_list_the_package() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] | ["hold", ..] => Ok((String::new(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(
        calls,
        vec![vec!["showhold"], vec!["hold", "pkg"], vec!["showhold"]]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], true);
    assert_eq!(result["holds"], json!([]));
    assert!(
        result["msg"]
            .as_str()
            .is_some_and(|msg| msg.contains("pkg")),
        "{result}"
    );
    Ok(())
}

#[test]
fn hold_is_idempotent_when_showhold_already_lists_every_name() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "2")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok(("pkg\n".into(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(calls, vec![vec!["showhold"]]);
    assert_eq!(result["changed"], false);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg"]));
    Ok(())
}

/// Regression pin: a held package counts as installed (the engine's dpkg
/// reader already accepts `hold ok installed`), so nothing but the hold
/// report runs; the report itself comes from `apt-mark showhold`.
#[test]
fn held_package_counts_as_installed_and_reports_its_hold() -> Result<(), String> {
    let database = Arc::new(Mutex::new(
        "Package: pkg\nStatus: hold ok installed\nArchitecture: amd64\nVersion: 2\n\n".to_string(),
    ));
    let mut state = SystemState::with_dpkg_status(database);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok(("pkg\n".into(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(calls, vec![vec!["showhold"]]);
    assert_eq!(result["changed"], false);
    assert_eq!(result["holds"], json!(["pkg"]));
    Ok(())
}

#[test]
fn hold_in_check_mode_runs_nothing_and_reports_pending() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok((String::new(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(true),
        &mut state,
        &mut command,
    )?;
    assert_eq!(calls, vec![vec!["showhold"]]);
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!([]));
    Ok(())
}

/// A restored hold is only as good as the state the host reports: an
/// `apt-mark hold` that exits 0 without taking effect would otherwise
/// pass, because the later reconciliation reads the snapshot taken before
/// the install, so the holds are read back right after the restore.
#[test]
fn a_restored_hold_that_does_not_take_effect_fails_the_step() -> Result<(), String> {
    let (mut state, database) = host(&[("pkg", "1")]);
    let mut held = true;
    let mut command = |args: &[&str], _: &ExecContext| {
        match args {
            ["showhold"] => Ok((
                if held { "pkg\n".into() } else { String::new() },
                0,
                String::new(),
            )),
            ["unhold", ..] => {
                held = false;
                Ok((String::new(), 0, String::new()))
            }
            // apt-mark hold exits 0 but marks nothing.
            ["hold", ..] => Ok((String::new(), 0, String::new())),
            ["install", ..] => {
                *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
                Ok((summary(true), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], true);
    assert!(
        result["msg"]
            .as_str()
            .is_some_and(|message| message.contains("could not be restored")),
        "{result}"
    );
    Ok(())
}

/// The closed selector grammar is checked before any command, so a hold
/// never runs against a request apt could not satisfy.
#[test]
fn hold_refuses_unsupported_selectors_before_any_command() {
    let (mut state, _) = host(&[]);
    let error = run_with(
        &json!({"name": "pkg=1.*", "hold": true}),
        &context(false),
        &mut state,
        &mut |args, _| Err(format!("unexpected apt call {args:?}")),
    )
    .expect_err("refused");
    assert!(error.contains("unsupported package selector"), "{error}");
}

mod pinned;

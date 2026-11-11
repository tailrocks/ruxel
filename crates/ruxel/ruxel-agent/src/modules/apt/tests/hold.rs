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

fn host(packages: &[(&str, &str)]) -> (SystemState, Arc<Mutex<String>>) {
    let database = Arc::new(Mutex::new(status(packages)));
    (SystemState::with_dpkg_status(database.clone()), database)
}

fn calls_of(calls: &mut Vec<Vec<String>>, args: &[&str]) {
    calls.push(args.iter().map(ToString::to_string).collect::<Vec<_>>());
}

#[test]
fn hold_installs_then_marks_held_and_reports_holds() -> Result<(), String> {
    let (mut state, database) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok((String::new(), 0, String::new())),
            ["install", ..] => {
                *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
                Ok((summary(true), 0, String::new()))
            }
            ["hold", ..] => Ok((String::new(), 0, String::new())),
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
        vec![vec!["showhold"], vec!["install", "pkg=2"], vec!["hold", "pkg"]]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg"]));
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

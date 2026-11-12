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
fn hold_refuses_a_malformed_showhold_line() {
    let (mut state, _) = host(&[("pkg", "1")]);
    let error = run_with(
        &json!({"name": "pkg", "hold": true}),
        &context(false),
        &mut state,
        &mut |args, _| match args {
            ["showhold"] => Ok(("pkg\nlib*\n".into(), 0, String::new())),
            other => Err(format!("unexpected apt call {other:?}")),
        },
    )
    .expect_err("refused");
    assert!(error.contains("invalid package selector"), "{error}");
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

/// A pinned hold converges over an old hold: the host holds `pkg` at 1
/// and the step asks for `pkg=2`, so the transaction must move a held
/// package. Only the requested hold is lifted for the transaction and
/// restored right after; apt's refusal to move any other held package
/// stands, and `--allow-change-held-packages` is never passed.
#[test]
fn a_pinned_hold_moves_the_version_the_host_holds() -> Result<(), String> {
    let (mut state, database) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let mut held = true;
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
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
            ["hold", ..] => {
                held = true;
                Ok((String::new(), 0, String::new()))
            }
            ["install", ..] => {
                if held {
                    // apt's own refusal.
                    return Ok((
                        String::new(),
                        1,
                        "E: Held packages were changed and -y was used without --allow-change-held-packages".into(),
                    ));
                }
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
            vec!["unhold", "pkg"],
            vec!["install", "pkg=2"],
            vec!["hold", "pkg"]
        ]
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(result["holds"], json!(["pkg"]));
    Ok(())
}

/// The relaxation is for `hold` transactions alone: a plain install keeps
/// apt's refusal to move a held package, so a plan that could not know of
/// the hold still fails loudly instead of moving a frozen package.
#[test]
fn a_plain_install_is_not_allowed_to_change_held_packages() {
    let (mut state, _) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let error = run_with(
        &json!({"name": "pkg=2"}),
        &context(false),
        &mut state,
        &mut |args, _| {
            calls_of(&mut calls, args);
            Ok((
                String::new(),
                1,
                "E: Held packages were changed and -y was used without --allow-change-held-packages".into(),
            ))
        },
    )
    .expect_err("apt refuses to move a held package");
    assert_eq!(calls, vec![vec!["install", "pkg=2"]]);
    assert!(error.contains("apt install failed"), "{error}");
}

/// The relaxation is scoped to the requested holds: an unrelated held
/// dependency keeps apt's refusal, so the run never moves a frozen
/// package the workflow did not ask for. `--allow-change-held-packages`
/// is never passed — it would authorize the whole transaction to change
/// any held package.
#[test]
fn an_unrelated_held_package_is_never_lifted_for_a_hold_transaction() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1"), ("bar", "1")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok(("bar\n".into(), 0, String::new())),
            ["hold", ..] | ["unhold", ..] => Ok((String::new(), 0, String::new())),
            ["install", ..] => {
                assert!(
                    !args.contains(&"--allow-change-held-packages"),
                    "the run passed the transaction-wide relaxation"
                );
                // The transaction wants bar too: apt's own refusal.
                Ok((
                    String::new(),
                    1,
                    "E: Held packages were changed and -y was used without --allow-change-held-packages"
                        .into(),
                ))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let error = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )
    .expect_err("apt refuses to move the unrelated held bar");
    assert!(error.contains("apt install failed"), "{error}");
    // bar is never unheld: pkg is not held, so no hold is lifted at all.
    assert_eq!(calls, vec![vec!["showhold"], vec!["install", "pkg=2"]]);
    Ok(())
}

use super::hold::{calls_of, host, status};
use super::*;

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

/// The relaxation is scoped to the requested holds: an unrelated held
/// dependency keeps apt's refusal, so the run never moves a frozen
/// package the workflow did not ask for. `--allow-change-held-packages`
/// is never passed — it would authorize the whole transaction to change
/// any held package.
#[test]
fn an_unrelated_held_package_is_never_lifted_for_a_hold_transaction() {
    let (mut state, _) = host(&[("pkg", "1"), ("bar", "1")]);
    let mut calls = Vec::new();
    let mut command = |args: &[&str], _: &ExecContext| {
        calls_of(&mut calls, args);
        match args {
            ["showhold"] => Ok(("bar\n".into(), 0, String::new())),
            ["hold" | "unhold", ..] => Ok((String::new(), 0, String::new())),
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
}

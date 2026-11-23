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
            vec!["hold", "pkg"],
            // The restore is verified: the holds are read back.
            vec!["showhold"]
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
fn a_plain_install_is_not_allowed_to_change_held_packages() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1")]);
    let mut calls = Vec::new();
    let result = run_with(
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
    .map_err(|error| format!("the run hid the failure behind {error:?}"))?;
    assert_eq!(calls, vec![vec!["install", "pkg=2"]]);
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], false, "the held package was not moved");
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(message.contains("apt install failed"), "{message}");
    Ok(())
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
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )
    .map_err(|error| format!("the run hid the failure behind {error:?}"))?;
    assert_eq!(result["failed"], true);
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(message.contains("apt install failed"), "{message}");
    // bar is never unheld: pkg is not held, so no hold is lifted at all.
    assert_eq!(calls, vec![vec!["showhold"], vec!["install", "pkg=2"]]);
    Ok(())
}

/// The hold is restored even when the transaction cannot start: the
/// `install` call fails to spawn — a missing binary, a killed exec — and
/// the error it answers must not skip `apt-mark hold`, or the host keeps
/// the step's lifted hold after the run failed. The transaction's own
/// error is reported after the restore attempt.
#[test]
fn a_hold_is_restored_even_when_the_transaction_cannot_start() -> Result<(), String> {
    let (mut state, _database) = host(&[("pkg", "1")]);
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
            ["install", ..] => Err("exec apt-get: no such file or directory".into()),
            other => Err(format!("unexpected apt call {other:?}")),
        }
    };
    let result = run_with(
        &json!({"name": "pkg=2", "hold": true}),
        &context(false),
        &mut state,
        &mut command,
    )
    .map_err(|error| format!("the run hid the failure behind {error:?}"))?;
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], false, "the restored hold hid no change");
    let message = result["msg"].as_str().unwrap_or_default();
    assert!(message.contains("exec apt-get"), "{message}");
    assert_eq!(
        calls,
        vec![
            vec!["showhold"],
            vec!["unhold", "pkg"],
            vec!["install", "pkg=2"],
            vec!["hold", "pkg"],
            vec!["showhold"]
        ],
        "the hold is restored whatever the transaction answered"
    );
    assert!(held, "the host holds the package again");
    Ok(())
}

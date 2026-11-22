//! Check-mode shell validation consults the plan's own File and
//! Symlink providers: check mode runs no file creation, so a shell an
//! earlier step of the plan manages is still missing from the live
//! filesystem when the account's task verifies it — the provider
//! record excuses it the way `planned_group` excuses a group
//! `groupadd` has not created yet. The lowering orders every managed
//! shell provider before the account
//! (`linux-user-shell-symlink-chain-before-account.pkl` chains a file
//! through two links before it,
//! `linux-user-shell-reconciled-later-stage.pkl` provides the file an
//! earlier stage, and
//! `linux-user-shell-obsolete-provider-replaced.pkl` replaces the
//! earlier managers with an executable file before it), and apply runs
//! the provider first and verifies the live shell it created — so the
//! excuse holds in check mode only.

use super::super::*;

fn check_context() -> ExecContext {
    ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

fn apply_context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

#[test]
fn check_mode_consults_planned_file_and_symlink_providers() {
    // A File step of the plan manages the shell, and check mode runs
    // no file creation: the live filesystem still lacks the path when
    // the account's task verifies it, so the dry run must consult the
    // plan's own provider record instead of refusing a change the
    // apply performs — the identical apply deterministically succeeds,
    // the provider having run first.
    shell::reset_planned_shell_providers();
    let shell = format!("/tmp/ruxel-shell-planned-file-{}", std::process::id());
    assert!(
        std::fs::symlink_metadata(&shell).is_err(),
        "the planned shell must be missing from the live filesystem"
    );
    let account = format!("ruxel-shell-planned-file-{}", std::process::id());
    shell::note_planned_shell_provider(&shell);
    let planned = run(&json!({"name": account, "shell": shell}), &check_context())
        .expect("check mode excuses a shell an earlier File step provides");
    assert_eq!(planned.get("changed"), Some(&json!(true)), "{planned}");
    // A Symlink step manages the declared path and its target chain
    // the way `linux-user-shell-symlink-chain-before-account.pkl`
    // chains a file through two links before the account: every hop
    // is the plan's to provide while the live filesystem holds none
    // of them.
    let target = format!("/tmp/ruxel-shell-planned-real-{}", std::process::id());
    let link = format!("/tmp/ruxel-shell-planned-link-{}", std::process::id());
    assert!(std::fs::symlink_metadata(&link).is_err());
    shell::note_planned_shell_provider(&target);
    shell::note_planned_shell_link(&link, &target);
    let chained = run(
        &json!({"name": format!("ruxel-shell-planned-link-{}", std::process::id()), "shell": link}),
        &check_context(),
    )
    .expect("check mode excuses a shell an earlier Symlink step provides");
    assert_eq!(chained.get("changed"), Some(&json!(true)), "{chained}");
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_refuses_a_missing_shell_no_step_provides() {
    // A shell no File or Symlink step of the plan provides is refused
    // here the way apply refuses it: the identical apply
    // deterministically fails `shell_allowed` on the missing path, so
    // the dry run must report that instead of a valid planned change —
    // and a provider for another path excuses nothing.
    shell::reset_planned_shell_providers();
    let missing = format!("/tmp/ruxel-shell-unprovided-{}", std::process::id());
    assert!(std::fs::symlink_metadata(&missing).is_err());
    shell::note_planned_shell_provider(&format!("{missing}-elsewhere"));
    let error = run(
        &json!({"name": format!("ruxel-shell-unprovided-{}", std::process::id()), "shell": missing}),
        &check_context(),
    )
    .expect_err("check mode must refuse a missing shell no step provides");
    assert!(error.contains("shell"), "{error}");
    shell::reset_planned_shell_providers();
}

#[test]
fn apply_mode_verifies_the_live_shell_from_the_host() {
    // Apply runs the providing step first and resolves from the host
    // alone: a planned provider excuses nothing there — a shell still
    // missing when the account's task runs is refused before anything
    // mutates, the stand-in absorbing nothing. A live executable shell
    // instead applies, the stand-in absorbing the creation.
    use std::os::unix::fs::PermissionsExt;

    shell::reset_planned_shell_providers();
    let missing = format!("/tmp/ruxel-shell-apply-missing-{}", std::process::id());
    assert!(std::fs::symlink_metadata(&missing).is_err());
    shell::note_planned_shell_provider(&missing);
    test_hook::arm(vec![]);
    let error = run(
        &json!({"name": format!("ruxel-shell-apply-missing-{}", std::process::id()), "shell": missing}),
        &apply_context(),
    )
    .expect_err("apply must refuse a shell the live filesystem still lacks");
    assert!(error.contains("shell"), "{error}");
    assert!(
        test_hook::take_calls().is_empty(),
        "useradd never ran: the missing shell was refused before it"
    );
    // The provider ran: the live shell exists and is executable for
    // the account, so the apply succeeds.
    let live =
        std::path::Path::new("/tmp").join(format!("ruxel-shell-apply-live-{}", std::process::id()));
    std::fs::write(&live, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o755)).unwrap();
    test_hook::arm(vec![Ok(())]);
    let applied = run(
        &json!({"name": format!("ruxel-shell-apply-live-{}", std::process::id()), "shell": live.to_str().unwrap(), "create_home": false}),
        &apply_context(),
    )
    .expect("apply succeeds where the provider created the live shell");
    assert_eq!(applied.get("changed"), Some(&json!(true)), "{applied}");
    let calls = test_hook::take_calls();
    assert!(
        calls.iter().any(|(cmd, _)| cmd == "useradd"),
        "the creation ran: {calls:?}"
    );
    std::fs::remove_file(&live).unwrap();
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_registers_the_copy_task_that_provides_the_shell() {
    // Threads 284 and 289: the provider record must be written by the
    // File task itself when it runs — a `note_planned_shell_provider`
    // with no production caller leaves the registry empty, so check
    // mode refuses a workflow the apply performs. An executed `copy`
    // with an executable mode registers its destination, and the
    // account's check-mode task reads it instead of failing on the
    // live filesystem the copy has not mutated yet.
    use super::super::super::copy;
    shell::reset_planned_shell_providers();
    let dir = std::env::temp_dir().join(format!("ruxel-shell-wiring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("mysh");
    let dest = dest.to_str().unwrap().to_owned();
    assert!(std::fs::symlink_metadata(&dest).is_err());
    let mut state = crate::system_state::SystemState::default();
    copy::run(
        &json!({"dest": dest, "content": "#!/bin/sh\n", "mode": "0755"}),
        None,
        &check_context(),
        &mut state,
    )
    .expect("the check-mode copy succeeds");
    assert!(
        shell::planned_shell_provided(&dest),
        "the executed copy registers its destination"
    );
    let account = format!("ruxel-shell-wiring-copy-{}", std::process::id());
    let planned = run(&json!({"name": account, "shell": dest}), &check_context())
        .expect("check mode excuses the shell the executed copy provides");
    assert_eq!(planned.get("changed"), Some(&json!(true)), "{planned}");
    let _ = std::fs::remove_dir_all(dir);
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_registers_the_link_task_that_provides_the_shell() {
    // Threads 284 and 289, the Symlink half: an executed `file`
    // state=link task registers its destination the way the copy
    // does, so the account's check-mode task follows the plan's own
    // link instead of failing on the live filesystem.
    use super::super::super::file;
    shell::reset_planned_shell_providers();
    let dir = std::env::temp_dir().join(format!("ruxel-shell-wiring-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("mysh");
    let link = link.to_str().unwrap().to_owned();
    file::run(
        &json!({"path": link, "src": "/bin/sh", "state": "link"}),
        &check_context(),
    )
    .expect("the check-mode link succeeds");
    assert!(
        shell::planned_shell_provided(&link),
        "the executed link registers its destination"
    );
    let account = format!("ruxel-shell-wiring-link-{}", std::process::id());
    let planned = run(&json!({"name": account, "shell": link}), &check_context())
        .expect("check mode excuses the shell the executed link provides");
    assert_eq!(planned.get("changed"), Some(&json!(true)), "{planned}");
    let _ = std::fs::remove_dir_all(dir);
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_refuses_a_link_to_a_missing_target() {
    // Thread 303: a Symlink step registers its declared target
    // chain, not its bare existence — a link to a target nothing
    // provides is refused in check mode the way apply refuses it,
    // instead of reporting a change the apply cannot perform.
    use super::super::super::file;
    shell::reset_planned_shell_providers();
    let dir = std::env::temp_dir().join(format!("ruxel-shell-dangling-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("nowhere");
    let missing = missing.to_str().unwrap().to_owned();
    assert!(std::fs::symlink_metadata(&missing).is_err());
    let link = dir.join("mysh");
    let link = link.to_str().unwrap().to_owned();
    file::run(
        &json!({"path": link, "src": missing, "state": "link"}),
        &check_context(),
    )
    .expect("the check-mode link succeeds");
    let error = run(
        &json!({"name": format!("ruxel-shell-dangling-{}", std::process::id()), "shell": link}),
        &check_context(),
    )
    .expect_err("check mode must refuse a link to a target nothing provides");
    assert!(error.contains("shell"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_refuses_a_link_to_a_non_executable_target() {
    // Thread 303, the live-target half: a link to an existing but
    // non-executable file is no executable shell, so check mode
    // refuses it the way `shell_allowed` refuses it live.
    use super::super::super::file;
    use std::os::unix::fs::PermissionsExt;
    shell::reset_planned_shell_providers();
    let dir = std::env::temp_dir().join(format!("ruxel-shell-noexec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("nox");
    std::fs::write(&target, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (target, link) = (
        target.to_str().unwrap().to_owned(),
        dir.join("mysh").to_str().unwrap().to_owned(),
    );
    file::run(
        &json!({"path": link, "src": target, "state": "link"}),
        &check_context(),
    )
    .expect("the check-mode link succeeds");
    let error = run(
        &json!({"name": format!("ruxel-shell-noexec-{}", std::process::id()), "shell": link}),
        &check_context(),
    )
    .expect_err("check mode must refuse a link to a non-executable target");
    assert!(error.contains("shell"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
    shell::reset_planned_shell_providers();
}

#[test]
fn check_mode_excuses_a_link_chain_to_a_planned_file() {
    // Thread 303, the positive half: a link to a target the plan
    // itself provides is excused — the chain resolves through the
    // planned link to the planned file, both still missing live.
    use super::super::super::file;
    shell::reset_planned_shell_providers();
    let dir = std::env::temp_dir().join(format!("ruxel-shell-chain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("real");
    let target = target.to_str().unwrap().to_owned();
    let link = dir.join("mysh");
    let link = link.to_str().unwrap().to_owned();
    shell::note_planned_shell_provider(&target);
    file::run(
        &json!({"path": link, "src": target, "state": "link"}),
        &check_context(),
    )
    .expect("the check-mode link succeeds");
    let planned = run(
        &json!({"name": format!("ruxel-shell-chain-{}", std::process::id()), "shell": link}),
        &check_context(),
    )
    .expect("check mode excuses a link to a target the plan provides");
    assert_eq!(planned.get("changed"), Some(&json!(true)), "{planned}");
    let _ = std::fs::remove_dir_all(dir);
    shell::reset_planned_shell_providers();
}

#[test]
fn a_new_plan_clears_the_shell_provider_record() {
    // A provider an earlier plan managed is not the next plan's to
    // rely on: `begin_plan` clears the shell-provider record beside
    // the identity one, so a later plan verifies against the live
    // filesystem again.
    shell::reset_planned_shell_providers();
    let shell = format!("/tmp/ruxel-shell-plan-reset-{}", std::process::id());
    shell::note_planned_shell_provider(&shell);
    assert!(shell::planned_shell_provided(&shell));
    begin_plan();
    assert!(
        !shell::planned_shell_provided(&shell),
        "a new plan starts with no shell provider"
    );
}

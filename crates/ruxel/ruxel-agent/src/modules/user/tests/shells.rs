//! The shell's path the signing-in account must be able to traverse.

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

#[test]
fn a_shell_beneath_an_unsearchable_directory_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    // `/root/bin/zsh` at mode 0755 beneath a `0700` `/root`: the
    // root-run lookup succeeds and the file's other-execute bit admits
    // it, but the managed account cannot traverse the directory on
    // login. Every directory component must be searchable by the
    // account — the conservative other-search bit, the same class the
    // file itself is judged by — before the shell is accepted.
    let dir = std::path::Path::new("/tmp").join(format!("ruxel-shell-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let shell = dir.join("zsh");
    std::fs::write(&shell, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell": shell.to_str().unwrap()}),
        &check_context(),
    )
    .expect_err("a shell the account cannot reach must be refused");
    assert!(error.contains("shell"), "{error}");
    // The same other-executable shell beneath a directory every
    // account searches is accepted: only the traversal was missing.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(
        &json!({"name":"ruxel-synthetic-user", "shell": shell.to_str().unwrap()}),
        &check_context(),
    )
    .expect("a shell beneath a searchable directory must be accepted");
    std::fs::remove_dir_all(&dir).unwrap();
}

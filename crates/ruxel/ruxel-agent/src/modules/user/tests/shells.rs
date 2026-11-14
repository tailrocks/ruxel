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

#[test]
fn a_shell_symlink_chain_reaches_its_target_directories() {
    use std::os::unix::fs::PermissionsExt;
    // A chain of links is the account's walk too: `mysh -> mid ->
    // <root>/target/deep/mysh` names no directory of the target in
    // the declared path, and the unsearchable one is `deep` — the
    // second directory of the target — so every traversed directory
    // is judged, not the target's first step alone.
    let root =
        std::path::Path::new("/tmp").join(format!("ruxel-shell-chain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let visible = root.join("visible");
    let deep = root.join("target").join("deep");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::create_dir(&visible).unwrap();
    std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = deep.join("mysh");
    std::fs::write(&target, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&target, visible.join("mid")).unwrap();
    let shell = visible.join("mysh");
    std::os::unix::fs::symlink("mid", &shell).unwrap();
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell": shell.to_str().unwrap()}),
        &check_context(),
    )
    .expect_err("a chain reaching an unsearchable target directory must be refused");
    assert!(error.contains("shell"), "{error}");
    std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(
        &json!({"name":"ruxel-synthetic-user", "shell": shell.to_str().unwrap()}),
        &check_context(),
    )
    .expect("a chain whose directories every account can walk must be accepted");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_shell_symlinked_directory_walks_its_target_directories() {
    use std::os::unix::fs::PermissionsExt;
    // The link may stand in a directory of the path, not at the
    // shell: `visible/up -> <root>/unsearch/sub` sends the walk
    // through `unsearch`, which no component of the declared path
    // names and the lexical ancestors' check never reads — the link
    // resolves to `sub`, whose own mode is searchable, while
    // `unsearch` above it is not.
    let root = std::path::Path::new("/tmp").join(format!("ruxel-shell-mid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let visible = root.join("visible");
    let sub = root.join("unsearch").join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir(&visible).unwrap();
    std::fs::set_permissions(
        root.join("unsearch"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let shell = sub.join("mysh");
    std::fs::write(&shell, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&sub, visible.join("up")).unwrap();
    let declared = visible.join("up").join("mysh");
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell": declared.to_str().unwrap()}),
        &check_context(),
    )
    .expect_err("a link's target directory walk must be refused where it cannot be searched");
    assert!(error.contains("shell"), "{error}");
    std::fs::set_permissions(
        root.join("unsearch"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    run(
        &json!({"name":"ruxel-synthetic-user", "shell": declared.to_str().unwrap()}),
        &check_context(),
    )
    .expect("a link's target directory walk must be accepted where it can be searched");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_shell_symlink_target_beneath_an_unsearchable_directory_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    // `/usr/local/bin/mysh -> /root/bin/mysh`: root's lookup follows
    // the link to the executable file and the lexical ancestors of the
    // link are searchable, but the managed account must also traverse
    // every directory the chain leads through. The chain is followed
    // hop by hop — a second link behind the first — and a symlinked
    // ancestor of the target counts too.
    let dir = std::path::Path::new("/tmp").join(format!("ruxel-shell-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let links = dir.join("links");
    let closed = dir.join("closed");
    let bin = closed.join("bin");
    std::fs::create_dir_all(&links).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = bin.join("mysh");
    std::fs::write(&target, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    let shell = links.join("mysh");
    std::os::unix::fs::symlink(&target, &shell).unwrap();
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell": shell.to_str().unwrap()}),
        &check_context(),
    )
    .expect_err("a shell whose target the account cannot reach must be refused");
    assert!(error.contains("not searchable"), "{error}");
    // The complete chain: a link behind the link is followed too.
    let first = links.join("first");
    std::os::unix::fs::symlink(&shell, &first).unwrap();
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell": first.to_str().unwrap()}),
        &check_context(),
    )
    .expect_err("a chain of two links is followed to its end");
    assert!(error.contains("not searchable"), "{error}");
    // Only the traversal was missing: open the directory and both
    // shells are accepted.
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(
        &json!({"name":"ruxel-synthetic-user", "shell": first.to_str().unwrap()}),
        &check_context(),
    )
    .expect("a reachable chain must be accepted");
    std::fs::remove_dir_all(&dir).unwrap();
}

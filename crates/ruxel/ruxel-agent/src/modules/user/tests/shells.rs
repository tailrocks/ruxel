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

#[test]
fn chain_intermediates_trust_only_privileged_group_write() {
    // Synthetic (uid, gid, mode) tuples pinning the chain rule: the
    // strict base, the privileged-group exception, and the refusals.
    // `wheel` is gid 0 everywhere (privileged); 50 is unprivileged on
    // every platform; macOS admin is 80 and privilege-equivalent,
    // while gid 80 elsewhere is an ordinary group.
    let operator = adoption::operating_uid();
    let stranger = if operator == 60000 { 59999 } else { 60000 };
    // The strict base admits clean operator-owned directories ...
    assert!(adoption::chain_dir_trusted(operator, 0, 0o755));
    assert!(adoption::chain_dir_trusted(0, 0, 0o755));
    // ... and on Linux nothing else admits: a lower-privileged member
    // of the owning group can rename the target aside and plant a
    // replacement home, so even root:root 0775 refuses there — while
    // off Linux the exception still admits root-owned group
    // writability by a privileged group. Both platform rules are
    // pinned explicitly so the Linux one is proven on any host.
    assert!(
        !adoption::chain_dir_trusted_on(0, 0, 0o775, true),
        "root:root 0775 refuses on Linux"
    );
    assert!(
        adoption::chain_dir_trusted_on(0, 0, 0o775, false),
        "root:wheel admits off Linux"
    );
    #[cfg(target_os = "macos")]
    assert!(
        adoption::chain_dir_trusted(0, 80, 0o775),
        "root:admin admits on macOS"
    );
    #[cfg(not(target_os = "macos"))]
    assert!(
        !adoption::chain_dir_trusted(0, 80, 0o775),
        "gid 80 is ordinary off macOS"
    );
    // An unprivileged group refuses, by name where the host spells it.
    assert!(
        !adoption::chain_dir_trusted(0, 50, 0o775),
        "root:staff refuses"
    );
    if let Some(staff) = staff_gid().filter(|gid| !adoption::privileged_gid(*gid)) {
        assert!(
            !adoption::chain_dir_trusted(0, staff, 0o775),
            "root:staff refuses"
        );
    }
    // A non-root owner with group-write refuses whatever the group.
    assert!(!adoption::chain_dir_trusted(stranger, 0, 0o775));
    assert!(!adoption::chain_dir_trusted(stranger, 80, 0o775));
    // Other-write refuses for every owner and group.
    for (uid, gid) in [(0, 0), (0, 80), (stranger, 20)] {
        assert!(!adoption::chain_dir_trusted(uid, gid, 0o777));
    }
}

/// The host's `staff` gid, where the group file spells one.
fn staff_gid() -> Option<u32> {
    std::fs::read_to_string("/etc/group")
        .ok()?
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.first() == Some(&"staff"))
                .then(|| f.get(2)?.parse::<u32>().ok())
                .flatten()
        })
}

/// A group the test process may `chgrp` to that holds no privileged
/// principal: the euid's own primary group (an owner may always chgrp
/// to it) where it is unprivileged, else a well-known unprivileged
/// group from the host's group file. `None` where the platform offers
/// neither — the fixture cannot be staged there.
fn unprivileged_chgrp_target() -> Option<(String, u32)> {
    if let Some(primary) = euid_primary_gid().filter(|gid| !adoption::privileged_gid(*gid)) {
        return Some(("primary".to_string(), primary));
    }
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    ["staff", "nobody", "nogroup"].into_iter().find_map(|name| {
        groups
            .lines()
            .find_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                (f.first() == Some(&name))
                    .then(|| f.get(2)?.parse::<u32>().ok())
                    .flatten()
            })
            .filter(|gid| !adoption::privileged_gid(*gid))
            .map(|gid| (name.to_string(), gid))
    })
}

/// The euid's primary group: the group a file the process creates
/// belongs to (see `operating_uid`).
fn euid_primary_gid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let probe = std::env::temp_dir().join(format!("ruxel-chain-gid-{}", std::process::id()));
    std::fs::write(&probe, b"").ok()?;
    let gid = std::fs::metadata(&probe).ok().map(|meta| meta.gid());
    let _ = std::fs::remove_file(&probe);
    gid
}

#[test]
fn an_unprivileged_group_writable_intermediate_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    // End to end through the real walk: the home continues beneath a
    // link whose target leads through a root-owned 0775 intermediate
    // held by an UNPRIVILEGED group — the walk must refuse it, while
    // the same chain with the intermediate closed admits.
    //
    // Root lane only: a non-root link parent never satisfies
    // `standing_parent`, so no non-root staging can isolate an
    // intermediate refusal — the walk refuses at the parent first.
    if adoption::operating_uid() != 0 {
        return;
    }
    let Some((_, gid)) = unprivileged_chgrp_target() else {
        return;
    };
    // Anchored directly under the root: every prefix above the staging
    // is root-owned and unwritable, so only the staged intermediate can
    // refuse.
    let base = std::path::Path::new("/").join(format!("ruxel-chain-unpriv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let real = base.join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::chown(&real, None, Some(gid)).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
    let home = base.join("link/alice");
    // Control first: the same chain with the intermediate closed to
    // the group admits — the missing leaf ends the walk — proving the
    // staging refuses for the intermediate's group-write, not the
    // environment.
    adoption::symlink_target_trusted(
        &base,
        std::path::Path::new("link/alice"),
        home.to_str().unwrap(),
    )
    .expect("a chain of closed directories is admitted");
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o775)).unwrap();
    let error = adoption::symlink_target_trusted(
        &base,
        std::path::Path::new("link/alice"),
        home.to_str().unwrap(),
    )
    .expect_err("an unprivileged group-writable intermediate must be refused");
    assert!(error.contains("writable by others"), "{error}");
    std::fs::remove_dir_all(&base).unwrap();
}

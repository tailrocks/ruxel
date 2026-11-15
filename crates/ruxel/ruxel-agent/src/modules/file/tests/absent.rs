use super::*;

fn scratch(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn check_mode_refuses_a_removal_of_a_nonempty_directory() {
    let base = scratch("nonempty");
    let dir = base.join("full");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("entry"), b"content").unwrap();
    let params = json!({"path": dir, "state": "absent"});
    let mut check = context();
    check.check_mode = true;
    // Check mode gives the same answer the run itself gives: a removal
    // without `recurse` takes an empty directory only.
    let check_error = run(&params, &check).unwrap_err();
    let run_error = run(&params, &context()).unwrap_err();
    assert_eq!(check_error, run_error);
    assert!(dir.exists(), "the directory is untouched");
    // With `recurse` the tree goes.
    let recursive = json!({"path": dir, "state": "absent", "recurse": true});
    assert_eq!(run(&recursive, &context()).unwrap()["changed"], true);
    assert!(!dir.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_reports_a_path_it_cannot_stat() {
    let base = scratch("stat-error");
    let file = base.join("file");
    std::fs::write(&file, b"content").unwrap();
    // A target below a regular file fails the read with something that
    // is not a missing path: the removal cannot establish the requested
    // state and must report the failure rather than answer for an
    // absence it never observed.
    let error = run(
        &json!({"path": file.join("sub"), "state": "absent"}),
        &context(),
    )
    .unwrap_err();
    assert!(error.contains("cannot read"), "{error}");
    assert!(error.contains("os error"), "{error}");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_of_an_empty_directory_takes_it() {
    let base = scratch("empty");
    let dir = base.join("empty");
    std::fs::create_dir(&dir).unwrap();
    let params = json!({"path": dir, "state": "absent"});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert!(!dir.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_reports_a_path_it_cannot_read() {
    let base = scratch("unreadable");
    // A symlink that points at itself: the path below it cannot be read
    // at all, and that is not the same as the path being absent.
    std::os::unix::fs::symlink("loop", base.join("loop")).unwrap();
    let params = json!({"path": base.join("loop/child"), "state": "absent"});
    let error = run(&params, &context()).unwrap_err();
    assert!(error.contains("file: cannot"), "{error}");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_walks_no_dot_segments() {
    let base = scratch("dot-segment");
    // `..` would name a directory the path never mentioned: the removal
    // refuses it instead of resolving it.
    let path = base.join("../etc");
    let mut check = context();
    check.check_mode = true;
    let error = run(
        &json!({"path": path, "state": "absent", "recurse": true}),
        &check,
    )
    .unwrap_err();
    assert!(error.contains("dot segments"), "{error}");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_does_not_follow_a_symlinked_ancestor() {
    let base = scratch("no-follow");
    let real = base.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("file"), b"safe").unwrap();
    // The ancestor is a link: the removal walks the path without
    // following it, so the link is refused and the tree behind it is
    // left alone.
    std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
    let params = json!({"path": base.join("link/file"), "state": "absent"});
    let error = run(&params, &context()).unwrap_err();
    assert!(error.contains("file: cannot"), "{error}");
    assert_eq!(std::fs::read(real.join("file")).unwrap(), b"safe");
    std::fs::remove_dir_all(&base).unwrap();
}

/// The removal of an absolute path does not depend on the working
/// directory of the agent: a working directory that has been renamed
/// away stops only a relative path.
///
/// The working directory is a property of the process, so the case runs
/// in a process of its own: the test binary is run again with the mark
/// set, and there the working directory is taken away before the
/// removal.
#[test]
fn a_removal_of_an_absolute_path_survives_a_deleted_working_directory() {
    const MARK: &str = "RUXEL_TEST_WORKING_DIRECTORY_GONE";
    if std::env::var_os(MARK).is_some() {
        removed_with_a_deleted_working_directory();
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "a_removal_of_an_absolute_path_survives_a_deleted_working_directory",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MARK, "1")
        .status()
        .unwrap();
    assert!(status.success(), "the removal in the child process failed");
}

/// The removal of an absolute path in a process whose working directory
/// is gone.
fn removed_with_a_deleted_working_directory() {
    let base = scratch("deleted-working-directory");
    let gone = base.join("gone");
    std::fs::create_dir(&gone).unwrap();
    let target = base.join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("file"), b"content").unwrap();
    let before = std::env::current_dir().unwrap();
    std::env::set_current_dir(&gone).unwrap();
    // The working directory is gone: the removal of an absolute path
    // does not need it.
    std::fs::remove_dir(&gone).unwrap();
    let result = run(
        &json!({"path": target, "state": "absent", "recurse": true}),
        &context(),
    );
    std::env::set_current_dir(before).unwrap();
    let result = result.unwrap();
    assert_eq!(result["changed"], true);
    assert!(!target.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_refuses_when_the_mount_points_of_the_host_cannot_be_read() {
    let base = scratch("no-mount-table");
    let dir = base.join("dir");
    std::fs::create_dir(&dir).unwrap();
    // The mount table of the host cannot be read: the removal cannot
    // tell a mount point from an ordinary directory, and it refuses
    // rather than walk a tree it cannot see the mounts of.
    let error = super::super::removal::remove_with_listing(&dir, true, false, &|| {
        Err("the mount table cannot be read".to_string())
    })
    .unwrap_err();
    assert!(error.contains("mount points of the host"), "{error}");
    // The tree is untouched.
    assert!(dir.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_of_a_tree_takes_everything_under_it() {
    let base = scratch("tree");
    let tree = base.join("tree");
    std::fs::create_dir_all(tree.join("nested")).unwrap();
    std::fs::write(tree.join("file"), b"content").unwrap();
    std::fs::write(tree.join("nested/file"), b"content").unwrap();
    std::os::unix::fs::symlink(tree.join("file"), tree.join("link")).unwrap();
    let params = json!({"path": tree, "state": "absent", "recurse": true});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert!(!tree.exists());
    assert!(base.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_of_a_path_below_a_missing_ancestor_is_no_change() {
    let base = scratch("missing-ancestor");
    // The ancestor is not there, so the target is not there either: the
    // removal is no change, not a failure.
    let params = json!({"path": base.join("missing/child"), "state": "absent"});
    assert_eq!(run(&params, &context()).unwrap()["changed"], false);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_relative_path_is_anchored_before_the_floor_holds_it() {
    let base = scratch("relative");
    // The removal anchors a relative path at the working directory of the
    // agent, and the floor holds the anchored path: the working directory
    // here is the scratch space, so the path is not a root.
    let params = json!({"path": "etc", "state": "absent", "recurse": true});
    let result = run(&params, &context());
    assert!(result.is_ok(), "the anchored path is not a root");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_mount_points_of_the_host_are_read_from_the_kernel() {
    // The kernel writes the characters that need it as octal escapes.
    assert_eq!(
        super::super::mounts::unescape_mount_bytes(b"/tmp/a\\040b"),
        b"/tmp/a b".to_vec()
    );
    assert_eq!(
        super::super::mounts::unescape_mount_bytes(b"/tmp/plain"),
        b"/tmp/plain".to_vec()
    );
    // The mount points of this host are read from the kernel, and they are
    // paths. The listing ends with an empty line, which is not a mount
    // point.
    for mount in super::super::mounts::mounts().unwrap() {
        let point = mount.point;
        assert!(!point.as_os_str().is_empty(), "{point:?}");
        assert!(point.is_absolute(), "{point:?}");
    }
}

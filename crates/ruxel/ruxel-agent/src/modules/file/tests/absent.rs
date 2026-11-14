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
fn a_removal_refuses_a_system_root() {
    let mut check = context();
    check.check_mode = true;
    for root in [
        "/", "/etc", "/var", "/root", "/home", "/usr", "/boot", "/dev", "/proc", "/sys",
    ] {
        let error = run(&json!({"path": root, "state": "absent"}), &check).unwrap_err();
        assert!(error.contains("system root"), "{root}: {error}");
    }
}

#[test]
fn a_removal_refuses_a_path_that_reaches_a_system_root() {
    let base = scratch("floor");
    // A symlinked ancestor carries the removal into the real /etc: the
    // path reads like a directory below the scratch space and lands on the
    // system's own.
    std::os::unix::fs::symlink("/", base.join("root")).unwrap();
    let path = base.join("root/etc");
    let mut check = context();
    check.check_mode = true;
    let error = run(
        &json!({"path": path, "state": "absent", "recurse": true}),
        &check,
    )
    .unwrap_err();
    assert!(error.contains("system root"), "{error}");
    // The system's own directory is untouched.
    assert!(std::path::Path::new("/etc").exists());
    std::fs::remove_dir_all(&base).unwrap();
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
fn a_removal_of_a_link_to_a_path_below_a_system_root_takes_the_link() {
    let base = scratch("floor-link-below");
    // The finding names both: a link to a system root, *or to a path
    // below one*, is a link, and taking the link takes nothing.
    std::os::unix::fs::symlink("/etc/hosts", base.join("old-hosts")).unwrap();
    let params = json!({"path": base.join("old-hosts"), "state": "absent"});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert!(base.join("old-hosts").symlink_metadata().is_err());
    assert!(std::path::Path::new("/etc/hosts").exists());
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
fn a_removal_refuses_a_path_that_reaches_a_system_root_below_it() {
    let base = scratch("floor-descendant");
    // The symlinked ancestor carries the removal into the real /etc, and
    // the target sits below the root the plan never named.
    std::os::unix::fs::symlink("/", base.join("root")).unwrap();
    let path = base.join("root/etc/subdir");
    let mut check = context();
    check.check_mode = true;
    let error = run(
        &json!({"path": path, "state": "absent", "recurse": true}),
        &check,
    )
    .unwrap_err();
    assert!(error.contains("system root"), "{error}");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_removal_of_a_link_to_a_system_root_takes_the_link() {
    let base = scratch("floor-link");
    // The last component is a link to a system root: removing the link
    // takes nothing of the root.
    std::os::unix::fs::symlink("/etc", base.join("old-etc")).unwrap();
    let params = json!({"path": base.join("old-etc"), "state": "absent"});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert!(!base.join("old-etc").exists());
    assert!(std::path::Path::new("/etc").exists());
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
fn a_directory_that_is_another_name_for_a_root_is_refused() {
    // A bind mount keeps the identity of the directory it is mounted
    // from, so the path says nothing about where the removal lands: the
    // same directory under another name is refused.
    assert!(super::super::removal::aliases_root(
        std::path::Path::new("/scratch/root"),
        (7, 11),
        std::path::Path::new("/"),
        (7, 11)
    ));
    // The root under its own name is the root the plan may name a path
    // below.
    assert!(!super::super::removal::aliases_root(
        std::path::Path::new("/etc"),
        (7, 11),
        std::path::Path::new("/etc"),
        (7, 11)
    ));
    // Another directory is another directory.
    assert!(!super::super::removal::aliases_root(
        std::path::Path::new("/scratch/root"),
        (7, 11),
        std::path::Path::new("/"),
        (7, 12)
    ));
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
fn a_removal_does_not_follow_a_symlinked_first_component() {
    let base = scratch("first-component");
    // The first component is a link to the filesystem root: the walk
    // follows a first component only when it is the system's own alias
    // for a directory of the same name, so this one is refused.
    let link = base.join("root");
    std::os::unix::fs::symlink("/", &link).unwrap();
    let params = json!({"path": link.join("etc"), "state": "absent", "recurse": true});
    let error = run(&params, &context()).unwrap_err();
    assert!(
        error.contains("file: refusing") || error.contains("file: cannot"),
        "{error}"
    );
    assert!(std::path::Path::new("/etc").exists());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_first_component_is_followed_only_when_it_is_the_system_s_own_alias() {
    let base = scratch("alias");
    let private = base.join("private");
    std::fs::create_dir_all(private.join("var")).unwrap();
    // The system's own alias points at a directory of the same name.
    std::os::unix::fs::symlink(private.join("var"), base.join("var")).unwrap();
    let held = std::fs::File::open(&base).unwrap();
    assert!(super::super::floor::is_alias(
        &held,
        std::ffi::OsStr::new("var")
    ));
    // A link that points somewhere else is not an alias.
    std::os::unix::fs::symlink("/", base.join("scratch")).unwrap();
    assert!(!super::super::floor::is_alias(
        &held,
        std::ffi::OsStr::new("scratch")
    ));
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_mount_point_in_a_tree_is_not_crossed() {
    // An entry of another device is a mount point, and the tree below it
    // belongs to somewhere else: the removal does not descend into it.
    assert!(super::super::floor::is_mount_point(1, 2));
    assert!(!super::super::floor::is_mount_point(1, 1));
}

#[test]
fn a_root_in_another_spelling_is_a_root() {
    // `//etc` names the same root as `/etc`, and the removal refuses it.
    let mut check = context();
    check.check_mode = true;
    let error = run(&json!({"path": "//etc", "state": "absent"}), &check).unwrap_err();
    assert!(error.contains("system root"), "{error}");
}

#[test]
fn a_system_link_is_a_system_root() {
    // The system's own links into the merged tree are the system's own
    // names: a removal that takes one breaks the host.
    for root in ["/bin", "/sbin", "/lib", "/lib64"] {
        let mut check = context();
        check.check_mode = true;
        let error = run(&json!({"path": root, "state": "absent"}), &check).unwrap_err();
        assert!(error.contains("system root"), "{root}: {error}");
    }
}

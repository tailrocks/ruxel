//! The floor of a removal: the roots a removal never takes, and the
//! paths that reach them.

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
fn a_removal_refuses_a_system_root_of_the_platform() {
    let mut check = context();
    check.check_mode = true;
    // The roots of the platform the agent also runs on: `/private` holds
    // the real `/etc` and `/var` of macOS, `/Users` is the home root of
    // the platform, and the rest hold the system itself.
    for root in [
        "/private",
        "/Users",
        "/Library",
        "/System",
        "/Applications",
        "/Volumes",
    ] {
        let error = run(
            &json!({"path": root, "state": "absent", "recurse": true}),
            &check,
        )
        .unwrap_err();
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
    let base = scratch("floor-alias");
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

#[test]
fn a_root_under_another_spelling_is_still_a_root() {
    // The spelling says nothing on a case-insensitive filesystem, so a
    // root is also held by the device and the inode of the path itself.
    let base = scratch("identity-root");
    let real = base.join("real");
    std::fs::create_dir(&real).unwrap();
    let root = real.to_string_lossy().to_string();
    let roots = [root.as_str()];
    // The root under its own name is refused.
    assert!(super::super::floor::refuse_root(&real, &roots).is_err());
    // The same directory under another spelling (`..` stays written, so
    // the string differs) is refused by identity.
    let other = base.join("real").join("..").join("real");
    assert_ne!(other.to_string_lossy(), root);
    let error = super::super::floor::refuse_root(&other, &roots).unwrap_err();
    assert!(error.contains("another spelling"), "{error}");
    // A symlink to the root is a link, not the root: taking it takes
    // nothing, so it passes.
    let link = base.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    super::super::floor::refuse_root(&link, &roots).unwrap();
    // Another directory is neither spelling nor identity.
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    super::super::floor::refuse_root(&elsewhere, &roots).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_root_reached_through_dotdot_is_still_a_root() {
    // `/..` is the filesystem root under another spelling: the string
    // differs, but the identity is the root's own.
    let error = super::super::floor::refuse_system_root(std::path::Path::new("/..")).unwrap_err();
    assert!(error.contains("another spelling"), "{error}");
}

#[test]
fn a_directory_a_root_resolves_to_is_refused() {
    // The root is written under a symlink spelling, and the candidate
    // names the directory it resolves to: it is the root under another
    // spelling, not another directory.
    let base = scratch("resolved-root");
    let real = base.join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink("real", base.join("alias")).unwrap();
    let alias = base.join("alias").to_string_lossy().to_string();
    let roots = [alias.as_str()];
    let error = super::super::floor::refuse_root(&real, &roots).unwrap_err();
    assert!(error.contains("under another spelling"), "{error}");
    // A link to the directory is still a link: taking it takes the link.
    let link = base.join("link");
    std::os::unix::fs::symlink("real", &link).unwrap();
    super::super::floor::refuse_root(&link, &roots).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}

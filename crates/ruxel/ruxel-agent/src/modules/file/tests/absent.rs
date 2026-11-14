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
fn a_removal_of_an_empty_directory_takes_it() {
    let base = scratch("empty");
    let dir = base.join("empty");
    std::fs::create_dir(&dir).unwrap();
    let params = json!({"path": dir, "state": "absent"});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert!(!dir.exists());
    std::fs::remove_dir_all(&base).unwrap();
}

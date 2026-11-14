use super::*;

#[test]
fn default_file_state_applies_attrs_and_rejects_missing() {
    let dir = std::env::temp_dir().join(format!("ruxel-file-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("existing");
    std::fs::write(&path, b"content").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let result = run(&json!({"path": path, "mode": "0600"}), &context()).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let missing = dir.join("missing");
    assert!(
        run(&json!({"path": missing}), &context())
            .unwrap_err()
            .contains("does not exist")
    );
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn recursive_attributes_detect_and_repair_nested_drift() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-recursive-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    let file = dir.join("nested/file");
    std::fs::write(&file, b"content").unwrap();
    let params = json!({"path": dir, "state": "directory", "mode": "0700", "recurse": true});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert_eq!(run(&params, &context()).unwrap()["changed"], false);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut check = context();
    check.check_mode = true;
    assert_eq!(run(&params, &check).unwrap()["changed"], true);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(run(&params, &context()).unwrap()["changed"], false);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn recursive_attributes_do_not_follow_symlinks() {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-symlink-{}", std::process::id()));
    let root = base.join("root");
    let external = base.join("external");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let target = external.join("file");
    std::fs::write(&target, b"safe").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&external, root.join("dirlink")).unwrap();
    std::os::unix::fs::symlink(&target, root.join("filelink")).unwrap();
    let params = json!({"path": root, "state": "directory", "mode": "0755", "recurse": true});
    run(&params, &context()).unwrap();
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&external).unwrap().permissions().mode() & 0o777,
        0o700
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn recursive_attributes_repair_unreadable_root() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-root-unreadable-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("file"), b"safe").unwrap();
    std::fs::set_permissions(root.join("file"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let held_file = std::fs::File::open(root.join("file")).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    let params = json!({"path": root, "state": "directory", "mode": "0700", "recurse": true});
    let mut check = context();
    check.check_mode = true;
    if let Ok(result) = run(&params, &check) {
        assert_eq!(result["changed"], true);
    }
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o000
    );
    assert_eq!(
        held_file.metadata().unwrap().permissions().mode() & 0o777,
        0o644
    );
    let inaccessible = cfg!(target_os = "macos") && std::fs::File::open(&root).is_err();
    let result = run(&params, &context());
    if inaccessible {
        assert!(result.is_err());
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o000
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    } else {
        assert_eq!(result.unwrap()["changed"], true);
        assert_eq!(
            std::fs::metadata(root.join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

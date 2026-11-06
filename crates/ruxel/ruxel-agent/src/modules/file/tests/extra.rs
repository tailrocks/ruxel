use super::*;

#[test]
fn opened_directory_stays_anchored_when_path_is_swapped() {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-swap-{}", std::process::id()));
    let root = base.join("root");
    let external = base.join("external");
    let moved = base.join("moved");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let victim = external.join("victim");
    std::fs::write(&victim, b"safe").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
    let fd = open_attrs(&root).unwrap();
    std::fs::rename(&root, &moved).unwrap();
    std::os::unix::fs::symlink(&external, &root).unwrap();
    let params = json!({"mode": "0755"});
    let mut changed = false;
    apply_directory(&fd, params.as_object().unwrap(), &mut changed, false).unwrap();
    assert_eq!(
        std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(open_attrs(&root).is_err());
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn recursive_attributes_repair_unreadable_file() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-unreadable-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("file");
    std::fs::write(&file, b"content").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let params = json!({"path": root, "state": "directory", "mode": "0700", "recurse": true});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o700
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn recursive_root_accepts_aliases_and_parent_components() {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-alias-{}", std::process::id()));
    let real = base.join("real");
    std::fs::create_dir_all(real.join("nested")).unwrap();
    let file = real.join("file");
    std::fs::write(&file, b"content").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&real, base.join("alias")).unwrap();
    let path = base.join("alias/nested/..");
    let params = json!({"path": path, "state": "directory", "mode": "0700", "recurse": true});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o700
    );
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn recursive_attributes_repair_unreadable_directory() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-dir-unreadable-{}", std::process::id()));
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("nested/file"), b"safe").unwrap();
    std::fs::set_permissions(root.join("nested"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let params = json!({"path": root, "state": "directory", "mode": "0700", "recurse": true});
    assert_eq!(run(&params, &context()).unwrap()["changed"], true);
    assert_eq!(
        std::fs::metadata(root.join("nested/file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    std::fs::remove_dir_all(root).unwrap();
}

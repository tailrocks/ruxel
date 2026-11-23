use super::*;

#[test]
fn unreadable_root_alias_resolves_from_held_parent() {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-root-alias-{}", std::process::id()));
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("file"), b"safe").unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::os::unix::fs::symlink("root", base.join("alias")).unwrap();
    let params =
        json!({"path": base.join("alias"), "state":"directory", "mode":"0700", "recurse":true});
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
    std::fs::remove_dir_all(base).unwrap();
}

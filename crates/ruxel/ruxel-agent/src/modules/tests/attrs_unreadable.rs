use super::*;

#[test]
fn unreadable_owner_file_and_alias_prefix_preserve_check_semantics() {
    let root = Root::new();
    let directory = root.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    let file = directory.join("file");
    std::fs::write(&file, "payload").unwrap();
    let alias = root.path().join("alias");
    symlink(&directory, &alias).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let attrs = json!({"mode":"0600"});
    let mut changed = false;
    #[cfg(target_vendor = "apple")]
    if std::fs::File::open(&file).is_err() {
        assert!(
            apply_attrs(
                &alias.join("file"),
                attrs.as_object().unwrap(),
                &mut changed,
                true
            )
            .is_err()
        );
        assert!(
            apply_attrs(
                &alias.join("file"),
                attrs.as_object().unwrap(),
                &mut changed,
                false
            )
            .is_err()
        );
        assert!(!changed);
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        return;
    }
    apply_attrs(
        &alias.join("../directory/file"),
        attrs.as_object().unwrap(),
        &mut changed,
        true,
    )
    .unwrap();
    assert!(changed);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0
    );
    apply_attrs(
        &alias.join("file"),
        attrs.as_object().unwrap(),
        &mut changed,
        false,
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

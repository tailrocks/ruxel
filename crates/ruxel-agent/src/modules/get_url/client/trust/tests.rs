use super::supported_macos_filesystem;

#[test]
fn macos_filesystem_policy_rejects_remote_and_unknown_backing() {
    assert!(supported_macos_filesystem(true, b"apfs\0"));
    assert!(supported_macos_filesystem(true, b"hfs\0"));
    assert!(!supported_macos_filesystem(false, b"apfs\0"));
    assert!(!supported_macos_filesystem(true, b"nfs\0"));
    assert!(!supported_macos_filesystem(true, b"osxfuse\0"));
    assert!(!supported_macos_filesystem(true, b"unknown\0"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_filesystem_policy_rejects_remote_overlay_and_unknown_backing() {
    for kind in [0xef53, 0x5846_5342, 0x9123_683e, 0x0102_1994, 0x8584_58f6] {
        assert!(super::supported_linux_filesystem(kind));
    }
    for kind in [0x6969, 0xff53_4d42, 0x6573_5546, 0x794c_7630, 0] {
        assert!(!super::supported_linux_filesystem(kind));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_acl_grants_are_rejected_despite_private_mode_bits() {
    use crate::modules::get_url::download_tests::Root;
    use exacl::{AclEntry, Flag, Perm};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = Root::new();
    let file = root.0.join("client");
    fs::write(&file, b"synthetic client").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    super::filesystem_and_acl(&file).unwrap();
    let grant = AclEntry::allow_group("everyone", Perm::WRITE, Flag::empty());
    exacl::setfacl(&[&file], &[grant], None).unwrap();
    assert_eq!(fs::metadata(&file).unwrap().mode() & 0o777, 0o700);
    assert!(super::filesystem_and_acl(&file).is_err());
    exacl::setfacl(&[&file], &[], None).unwrap();

    let grant = AclEntry::allow_group("everyone", Perm::DELETE_CHILD, Flag::FILE_INHERIT);
    exacl::setfacl(&[&root.0], &[grant], None).unwrap();
    assert_eq!(fs::metadata(&root.0).unwrap().mode() & 0o777, 0o700);
    assert!(super::filesystem_and_acl(&root.0).is_err());
    exacl::setfacl(&[&root.0], &[], None).unwrap();
}

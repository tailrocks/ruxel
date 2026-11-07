use super::{download_tests::Root, staging::Download};
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[test]
fn parent_must_be_directory_and_regular_file_is_unchanged() {
    let root = Root::new();
    let parent = root.0.join("file");
    fs::write(&parent, b"unchanged").unwrap();
    assert!(Download::new(&parent.join("result")).is_err());
    assert_eq!(fs::read(&parent).unwrap(), b"unchanged");
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[test]
fn write_search_only_parent_remains_supported() {
    let root = Root::new();
    let parent = root.0.join("parent");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
    let stage = Download::new(&parent.join("result"));
    // Restore traversal for assertion/panic cleanup; acquisition occurred without read.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let stage = stage.unwrap();
    stage.publish(&parent.join("result")).unwrap();
    assert_eq!(fs::read(parent.join("result")).unwrap(), b"");
}

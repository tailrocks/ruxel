#![allow(clippy::unwrap_used, reason = "local fixture setup must succeed")]

use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(crate::mux::test_directory().unwrap())
    }

    fn file(&self) -> PathBuf {
        self.0.join("known")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn identity_requires_existing_trusted_regular_file() {
    let fixture = Fixture::new();
    let path = fixture.file();
    assert!(identity(&path).is_err());
    fs::write(&path, b"key").unwrap();
    assert!(identity(&path).is_ok());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o622)).unwrap();
    assert!(identity(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let link = fixture.0.join("link");
    symlink(&path, &link).unwrap();
    assert!(identity(&link).is_err());
    assert!(identity(&fixture.0).is_err());
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(identity(&path).is_err());
}

#[test]
fn precreates_private_file_and_keeps_existing_keys() {
    let fixture = Fixture::new();
    let path = fixture.file();
    known_hosts(&path).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    fs::write(&path, b"existing host keys\n").unwrap();
    let before = fs::metadata(&path).unwrap();
    known_hosts(&path).unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(fs::read(&path).unwrap(), b"existing host keys\n");
}

#[test]
fn rejects_writable_file_and_ancestor_without_repairing_them() {
    let fixture = Fixture::new();
    let path = fixture.file();
    fs::write(&path, b"existing").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o620)).unwrap();
    assert!(known_hosts(&path).is_err());
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o620);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(known_hosts(&path).is_err());
    assert!(known_hosts(&fixture.0.join("missing")).is_err());
    assert!(!fixture.0.join("missing").exists());
}

#[test]
fn rejects_final_symlink_directory_and_symlink_ancestor() {
    let fixture = Fixture::new();
    let path = fixture.file();
    fs::write(&path, b"existing").unwrap();
    let link = fixture.0.join("link");
    symlink(&path, &link).unwrap();
    assert!(known_hosts(&link).is_err());
    assert!(known_hosts(&fixture.0).is_err());
    fs::remove_file(&link).unwrap();
    symlink(&fixture.0, &link).unwrap();
    assert!(known_hosts(&link.join("known")).is_err());
    assert!(known_hosts(&link.join("missing")).is_err());
    assert!(!fixture.0.join("missing").exists());
}

#[test]
fn rejects_parent_traversal_before_creation() {
    let fixture = Fixture::new();
    let dir = fixture.0.join("child");
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    assert!(known_hosts(&dir.join("../known")).is_err());
    assert!(!fixture.file().exists());
}

mod platform;

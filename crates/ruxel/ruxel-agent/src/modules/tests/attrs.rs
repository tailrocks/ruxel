use super::{apply_attrs, apply_attrs_fd, open_attrs};
use serde_json::json;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn descriptor_attributes_survive_alias_retarget() {
    let root = Root::new();
    let original = root.path().join("original");
    let victim = root.path().join("victim");
    std::fs::write(&original, "original").unwrap();
    std::fs::write(&victim, "victim").unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
    let fd = open_attrs(&original).unwrap();
    std::fs::rename(&original, root.path().join("held")).unwrap();
    symlink(&victim, &original).unwrap();
    let attrs = json!({"mode":"0640"});
    let mut changed = false;
    apply_attrs_fd(&fd, attrs.as_object().unwrap(), &mut changed, false).unwrap();
    assert!(changed);
    assert_eq!(
        std::fs::metadata(root.path().join("held"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[path = "attrs_unreadable.rs"]
mod unreadable;

#[test]
fn terminal_dot_preserves_authored_directory_resolution() {
    let root = Root::new();
    let directory = root.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let alias = root.path().join("alias");
    symlink(&directory, &alias).unwrap();
    let attrs = json!({"mode":"0750"});
    for path in [
        alias.join("."),
        std::path::PathBuf::from(format!("{}/", alias.display())),
    ] {
        let mut changed = false;
        apply_attrs(&path, attrs.as_object().unwrap(), &mut changed, false).unwrap();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
}

#[test]
fn regular_file_with_directory_suffix_fails_without_mutation() {
    let root = Root::new();
    let file = root.path().join("file");
    std::fs::write(&file, "payload").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let attrs = json!({"mode":"0644"});
    let mut changed = false;
    let path = std::path::PathBuf::from(format!("{}/", file.display()));
    assert!(apply_attrs(&path, attrs.as_object().unwrap(), &mut changed, false).is_err());
    assert!(!changed);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn directory_access_modes_use_pinned_attributes() {
    let root = Root::new();
    let path = root.path().join("directory");
    std::fs::create_dir(&path).unwrap();
    let attrs = json!({"mode":"0700"});
    for mode in [0o111, 0o600] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let mut changed = false;
        apply_attrs(&path, attrs.as_object().unwrap(), &mut changed, true).unwrap();
        assert!(changed);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            mode
        );
        apply_attrs(&path, attrs.as_object().unwrap(), &mut changed, false).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

struct Root(std::path::PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruxel-attrs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

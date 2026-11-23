use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn a_file_that_was_replaced_after_it_was_read_is_not_unlinked() {
    let base = scratch("replaced-file");
    let file = base.join("tool");
    std::fs::write(&file, b"content").unwrap();
    let meta = std::fs::symlink_metadata(&file).unwrap();
    let identity = (meta.dev(), meta.ino());
    let parent = OwnedFd::from(std::fs::File::open(&base).unwrap());
    // The file is renamed away, and a replacement takes its name.
    std::fs::rename(&file, base.join("moved")).unwrap();
    std::fs::write(&file, b"replacement").unwrap();
    // The name does not hold the file that was read: the removal must
    // refuse, and the file that was read is not the one that goes.
    let error =
        super::super::removal::take_file(&parent, &CString::new("tool").unwrap(), identity, &file)
            .unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    // The replacement is back under the name it took, and the file that
    // was read is where it was.
    assert_eq!(std::fs::read(&file).unwrap(), b"replacement");
    assert_eq!(std::fs::read(base.join("moved")).unwrap(), b"content");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_bind_source_outside_every_mount_point_is_not_a_mount_point() {
    let base = scratch("mount-identity");
    let file = base.join("tool");
    std::fs::write(&file, b"content").unwrap();
    let meta = std::fs::metadata(&file).unwrap();
    // The entry carries a mounted identity but stands outside every
    // mount point: it is the bind source the mount shows elsewhere
    // (`tool` for `/usr/bin/tool`), not the mount, so the removal
    // takes it.
    let mounts = [super::super::mounts::Mount {
        root: PathBuf::from("/usr/bin"),
        point: PathBuf::from("/usr/bin/tool"),
        identity: (meta.dev(), meta.ino()),
    }];
    assert!(super::super::removal::remove_with(&file, false, false, &mounts).unwrap());
    assert!(!file.exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_target_that_a_mount_covers_is_refused() {
    let base = scratch("covered-target");
    let file = base.join("tool");
    std::fs::write(&file, b"content").unwrap();
    let meta = std::fs::metadata(&file).unwrap();
    // The target lies at the mount point itself: it is mounted content,
    // and the removal does not take it.
    let mounts = [super::super::mounts::Mount {
        root: PathBuf::from("/usr/bin"),
        point: file.clone(),
        identity: (meta.dev(), meta.ino()),
    }];
    let error = super::super::removal::remove_with(&file, false, false, &mounts).unwrap_err();
    assert!(error.contains("mount point"), "{error}");
    assert!(file.exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_mount_installed_after_the_first_read_is_read_again() {
    let base = scratch("late-bind");
    let root = base.join("root");
    std::fs::create_dir_all(root.join("opt")).unwrap();
    let target = root.join("opt/data");
    std::fs::write(&target, b"content").unwrap();
    // The filesystem root is mounted at the scratch root after the first
    // read: the descriptor walk enters the new mount, and the directory
    // that holds the target is the real `/opt` under another name.
    let meta = std::fs::metadata("/").unwrap();
    let mounts = [super::super::mounts::Mount {
        root: PathBuf::from("/"),
        point: root.clone(),
        identity: (meta.dev(), meta.ino()),
    }];
    // The parent is already held when this table is read: reading the
    // covering mount again is what refuses the real system file.
    let error = super::super::removal::remove_with(&target, true, false, &mounts).unwrap_err();
    assert!(error.contains("bound to /"), "{error}");
    assert!(target.exists(), "the system file is untouched");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn check_mode_without_a_mount_table_walks_the_same_boundary() {
    use std::os::unix::fs::PermissionsExt;
    // A platform that writes no mount table (macOS) lists no mounts, so
    // the preflight is skipped: the check must still walk the tree the
    // way the run walks it. A child the check cannot enter is a removal
    // the run cannot take, so the check refuses it too.
    let base = scratch("check-no-table");
    let sub = base.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("file"), b"content").unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o0)).unwrap();
    let empty = || Ok(Vec::new());
    let error = super::super::removal::remove_with_listing(&base, true, true, &empty).unwrap_err();
    assert!(
        !error.is_empty(),
        "the check refuses what the run cannot take"
    );
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(super::super::removal::remove_with_listing(&base, true, true, &empty).unwrap());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_parent_replaced_after_the_mutation_is_not_reported_as_done() {
    // The held directory is renamed away and replaced after it was
    // opened: the removal takes the old entry while the visible path
    // holds the replacement, so reporting success would name the wrong
    // tree.
    let base = scratch("detached-parent");
    let target = base.join("target");
    std::fs::write(&target, b"content").unwrap();
    let parent = OwnedFd::from(std::fs::File::open(&base).unwrap());
    let name = CString::new("target").unwrap();
    std::fs::rename(&base, base.with_extension("moved")).unwrap();
    std::fs::create_dir(&base).unwrap();
    std::fs::write(base.join("target"), b"replacement").unwrap();
    let mounts: Vec<super::super::mounts::Mount> = Vec::new();
    let error =
        super::super::removal::remove_opened(&parent, &name, &target, false, false, &mounts)
            .unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    // The old entry went through the held descriptor, but the visible
    // replacement is untouched.
    assert_eq!(std::fs::read(base.join("target")).unwrap(), b"replacement");
    assert!(!base.with_extension("moved").join("target").exists());
    std::fs::remove_dir_all(base.with_extension("moved")).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_detached_parent_is_not_reported_as_unchanged() {
    // The held directory is renamed away and replaced, and the old
    // directory holds no target: the visible path holds a replacement,
    // so "unchanged" would name the wrong tree.
    let base = scratch("detached-missing");
    let target = base.join("target");
    let parent = OwnedFd::from(std::fs::File::open(&base).unwrap());
    let name = CString::new("target").unwrap();
    std::fs::rename(&base, base.with_extension("moved")).unwrap();
    std::fs::create_dir(&base).unwrap();
    std::fs::write(base.join("target"), b"replacement").unwrap();
    let mounts: Vec<super::super::mounts::Mount> = Vec::new();
    let error =
        super::super::removal::remove_opened(&parent, &name, &target, false, false, &mounts)
            .unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    assert_eq!(std::fs::read(base.join("target")).unwrap(), b"replacement");
    std::fs::remove_dir_all(base.with_extension("moved")).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_attached_parent_verifies() {
    let base = scratch("attached-parent");
    let parent = OwnedFd::from(std::fs::File::open(&base).unwrap());
    super::super::removal::recheck::verify_attached(&parent, &base.join("target")).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}

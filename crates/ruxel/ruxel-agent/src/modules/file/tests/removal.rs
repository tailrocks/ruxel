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
fn a_target_that_carries_the_identity_of_a_mount_point_is_refused() {
    let base = scratch("mount-identity");
    let file = base.join("tool");
    std::fs::write(&file, b"content").unwrap();
    let meta = std::fs::metadata(&file).unwrap();
    // The mount point is named elsewhere, and the path does not name it:
    // the path may reach it through an alias (`/bin/tool` for
    // `/usr/bin/tool`), and the identity of the opened target is what
    // tells.
    let mounts = [super::super::mounts::Mount {
        root: PathBuf::from("/usr/bin"),
        point: PathBuf::from("/usr/bin/tool"),
        identity: (meta.dev(), meta.ino()),
    }];
    let error = super::super::removal::remove_with(&file, false, false, &mounts).unwrap_err();
    assert!(error.contains("mount point"), "{error}");
    // The tree of the mount point is not the removal's to take.
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

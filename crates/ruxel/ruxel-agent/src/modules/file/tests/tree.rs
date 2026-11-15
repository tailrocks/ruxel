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
fn a_nested_directory_that_was_replaced_is_not_emptied() {
    let base = scratch("replaced-nested");
    let dir = base.join("sub");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("file"), b"content").unwrap();
    let meta = std::fs::symlink_metadata(&dir).unwrap();
    let identity = (meta.dev(), meta.ino());
    let parent = OwnedFd::from(std::fs::File::open(&base).unwrap());
    let child = OwnedFd::from(std::fs::File::open(&dir).unwrap());
    // The directory is renamed away, and a replacement takes its name.
    std::fs::rename(&dir, base.join("moved")).unwrap();
    std::fs::create_dir(&dir).unwrap();
    // The name does not hold the directory that was opened: the removal
    // must refuse, and the tree of the moved directory stays.
    let error = super::super::tree::take_nested(
        &parent,
        &CString::new("sub").unwrap(),
        &child,
        identity,
        meta.dev(),
        &dir,
        &mut std::collections::HashSet::new(),
    )
    .unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    // The replacement is back under the name it took, and the tree that
    // was read is where it was.
    assert!(dir.exists());
    assert_eq!(std::fs::read(base.join("moved/file")).unwrap(), b"content");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn an_entry_with_the_staging_prefix_is_removed_like_any_other() {
    let base = scratch("staging-prefix");
    // The prefix reserves nothing: an entry a user named this way is an
    // entry like any other, and the walk takes it with the tree.
    std::fs::write(base.join(".ruxel-remove-1-1"), b"content").unwrap();
    std::fs::write(base.join("other"), b"content").unwrap();
    let dir = OwnedFd::from(std::fs::File::open(&base).unwrap());
    super::super::tree::remove_tree(&dir, &base).unwrap();
    assert!(
        !base.join(".ruxel-remove-1-1").exists(),
        "the prefixed entry is gone"
    );
    assert!(!base.join("other").exists(), "the other entry is gone");
    drop(dir);
    std::fs::remove_dir_all(base).unwrap();
}

use super::*;
use crate::modules::authorized_key::{
    ensure_attached, open_ssh, publish, read_authorized_keys, update,
};
use std::fs::File;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

#[test]
fn refuses_symlinks_and_invalid_content() {
    let root = scratch("refuse");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let victim = root.join("victim");
    std::fs::write(&victim, "untouched").unwrap();
    symlink(&victim, root.join(".ssh")).unwrap();
    assert!(open_ssh(&home, meta.uid(), meta.gid(), true).is_err());
    std::fs::remove_file(root.join(".ssh")).unwrap();
    std::fs::create_dir(root.join(".ssh")).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), false)
        .unwrap()
        .unwrap();
    symlink(&victim, root.join(".ssh/authorized_keys")).unwrap();
    assert!(read_authorized_keys(&ssh).is_err());
    std::fs::remove_file(root.join(".ssh/authorized_keys")).unwrap();
    // A comment may hold bytes that are not text: the file is read as
    // bytes, so such a file is read and not refused.
    std::fs::write(root.join(".ssh/authorized_keys"), [0xff]).unwrap();
    assert_eq!(read_authorized_keys(&ssh).unwrap(), vec![0xff]);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_swap_cannot_redirect_write() {
    let root = scratch("swap");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    std::fs::rename(root.join(".ssh"), root.join("original")).unwrap();
    let victim = root.join("victim");
    std::fs::create_dir(&victim).unwrap();
    std::fs::write(victim.join("authorized_keys"), "untouched").unwrap();
    symlink(&victim, root.join(".ssh")).unwrap();
    assert!(ensure_attached(&home, &ssh).is_err());
    publish(&ssh, b"new key\n", meta.uid(), meta.gid()).unwrap();
    assert_eq!(
        std::fs::read_to_string(victim.join("authorized_keys")).unwrap(),
        "untouched"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("original/authorized_keys")).unwrap(),
        "new key\n"
    );
    assert_eq!(
        std::fs::metadata(root.join("original/authorized_keys"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_swap_cannot_write_through_symlink() {
    let root = scratch("file-swap");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    assert_eq!(read_authorized_keys(&ssh).unwrap(), Vec::<u8>::new());
    let victim = root.join("victim");
    std::fs::write(&victim, "untouched").unwrap();
    symlink(&victim, root.join(".ssh/authorized_keys")).unwrap();
    publish(&ssh, b"key\n", meta.uid(), meta.gid()).unwrap();
    assert_eq!(std::fs::read_to_string(victim).unwrap(), "untouched");
    assert_eq!(
        std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
        "key\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_updates_preserve_both_keys() {
    let root = scratch("concurrent");

    let meta = std::fs::metadata(&root).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = ["ssh-ed25519 FIRST first", "ssh-ed25519 SECOND second"]
        .into_iter()
        .map(|key| {
            let root = root.clone();
            let barrier = barrier.clone();
            let meta = meta.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assert!(update(&root, meta.uid(), meta.gid(), key, false).unwrap());
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let keys = std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap();
    assert!(
        keys.contains("ssh-ed25519 FIRST first\n") && keys.contains("ssh-ed25519 SECOND second\n")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failed_write_leaves_no_staging_behind() {
    if rustix::process::geteuid().is_root() {
        // The write below fails at the ownership change, which only
        // fails for a writer that is not root.
        return;
    }
    let root = scratch("failed-write");
    let meta = std::fs::metadata(&root).unwrap();
    std::fs::create_dir(root.join(".ssh")).unwrap();
    let ssh = File::open(root.join(".ssh")).unwrap();
    // The owner the file is chowned to is not this writer: the staged
    // write fails after the staged entry is created. The partial entry
    // must go with the staging directory instead of stranding a
    // root-owned `.ruxel-key-*` the user cannot clean up.
    publish::publish_checked(&ssh, b"key\n", meta.uid() + 1, meta.gid(), None)
        .expect_err("the ownership change must fail");
    let staged: Vec<_> = std::fs::read_dir(root.join(".ssh"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".ruxel-key-")
        })
        .collect();
    assert!(staged.is_empty(), "staging left behind: {staged:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

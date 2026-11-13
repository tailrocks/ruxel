use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn material_ignores_comment() {
    assert_eq!(
        key_material("ssh-ed25519 AAAAC3Nza host-a"),
        key_material("ssh-ed25519 AAAAC3Nza other")
    );
    assert_eq!(
        key_material("from=\"10.0.0.1\" ssh-ed25519 AAAAC3Nza comment").unwrap(),
        ("ssh-ed25519".into(), "AAAAC3Nza".into())
    );
}

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
    std::fs::write(root.join(".ssh/authorized_keys"), [0xff]).unwrap();
    assert!(read_authorized_keys(&ssh).is_err());
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
    assert_eq!(read_authorized_keys(&ssh).unwrap(), "");
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

// The per-process counter keeps two tests that pick the same label from
// sharing one directory: `ruxel-file-swap-<pid>` is also a `file` test's
// scratch base, and a shared path made each test's `remove_dir_all` race
// the other's writes.
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ruxel-{label}-{}-{}",
        std::process::id(),
        NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    path
}

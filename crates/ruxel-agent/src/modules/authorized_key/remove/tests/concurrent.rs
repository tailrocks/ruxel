use super::*;
use crate::modules::authorized_key::remove::{BEFORE_WRITE, remove_key};

#[test]
fn a_file_that_changed_under_the_removal_is_not_overwritten() {
    let root = scratch("concurrent");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    std::fs::write(
        ssh.join("authorized_keys"),
        "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\n",
    )
    .unwrap();
    // A writer that does not take the lock adds a key after the
    // removal read the file: the removal must not write its own
    // snapshot over the newer file.
    let mine = ssh.join("authorized_keys");
    BEFORE_WRITE
        .lock()
        .unwrap()
        .push(std::sync::Arc::new(move |path: &std::path::Path| {
            if path != mine {
                return;
            }
            let mut content = std::fs::read_to_string(path).unwrap();
            content.push_str("ssh-ed25519 AAAAC3Nzaother\n");
            std::fs::write(path, content).unwrap();
        }));
    let result = remove_key(
        &root,
        meta.uid(),
        meta.gid(),
        "ssh-ed25519 AAAAC3Nzadrop",
        false,
    );
    let left = std::fs::read_to_string(ssh.join("authorized_keys")).unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    // The key the other writer added is still there.
    assert!(left.contains("AAAAC3Nzaother"), "{left}");
    assert!(left.contains("AAAAC3Nzadrop"), "{left}");
    std::fs::remove_dir_all(&root).unwrap();
}

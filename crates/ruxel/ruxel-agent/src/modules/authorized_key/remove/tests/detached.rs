use super::*;
use crate::modules::authorized_key::remove::{BEFORE_PUBLISH, remove_key};

#[test]
fn a_replaced_ssh_directory_is_noticeable_when_the_key_is_absent() {
    let _arming = ARMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = scratch("detached-absent");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // The directory this operation opened holds no key ...
    std::fs::write(ssh.join("authorized_keys"), "ssh-ed25519 AAAAC3Nzakeep\n").unwrap();
    // ... and the replacement holds the key that is to be absent.
    let replaced = root.clone();
    BEFORE_PUBLISH
        .lock()
        .unwrap()
        .push(std::sync::Arc::new(move |home: &std::path::Path| {
            if home != replaced {
                return;
            }
            std::fs::rename(replaced.join(".ssh"), replaced.join(".ssh-detached")).unwrap();
            std::fs::create_dir(replaced.join(".ssh")).unwrap();
            std::fs::write(
                replaced.join(".ssh/authorized_keys"),
                "ssh-ed25519 AAAAC3Nzadrop\n",
            )
            .unwrap();
        }));
    let result = remove_key(
        &root,
        meta.uid(),
        meta.gid(),
        "ssh-ed25519 AAAAC3Nzadrop",
        false,
    );
    let error = result.unwrap_err();
    assert!(error.contains(".ssh changed"), "{error}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_replaced_ssh_directory_stops_the_removal() {
    let _arming = ARMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = scratch("detached");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    std::fs::write(
        ssh.join("authorized_keys"),
        "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\n",
    )
    .unwrap();
    // The user replaces `.ssh` while the removal is between its check
    // and its write: the key stays in the file the user sees, and the
    // removal fails instead of reporting a change it did not make.
    let replaced = root.clone();
    BEFORE_PUBLISH
        .lock()
        .unwrap()
        .push(std::sync::Arc::new(move |home: &std::path::Path| {
            if home != replaced {
                return;
            }
            std::fs::rename(replaced.join(".ssh"), replaced.join(".ssh-detached")).unwrap();
            std::fs::create_dir(replaced.join(".ssh")).unwrap();
            std::fs::write(
                replaced.join(".ssh/authorized_keys"),
                "ssh-ed25519 AAAAC3Nzadrop\n",
            )
            .unwrap();
        }));
    let result = remove_key(
        &root,
        meta.uid(),
        meta.gid(),
        "ssh-ed25519 AAAAC3Nzadrop",
        false,
    );
    let error = result.unwrap_err();
    assert!(error.contains(".ssh changed"), "{error}");
    let left = std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap();
    assert!(left.contains("AAAAC3Nzadrop"), "{left}");
    std::fs::remove_dir_all(&root).unwrap();
}

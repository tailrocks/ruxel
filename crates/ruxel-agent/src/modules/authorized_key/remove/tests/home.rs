use super::*;
use crate::modules::authorized_key::remove::{BEFORE_PUBLISH, remove_key};

#[test]
fn a_missing_home_is_already_absent() {
    let root = scratch("home-missing");
    let meta = std::fs::metadata(&root).unwrap();
    // An account whose home is gone holds no key: a removal changes
    // nothing instead of failing on the open.
    let missing = root.join("no-such-home");
    assert!(
        !remove_key(
            &missing,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false
        )
        .unwrap()
    );
    assert!(
        !remove_key(
            &missing,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            true
        )
        .unwrap()
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_replaced_home_is_noticeable_when_the_key_is_absent() {
    let _arming = ARMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = scratch("home-detached-absent");
    // A failed run leaves the renamed home behind: clear it with the
    // scratch directory so the swap below always starts empty.
    let _ = std::fs::remove_dir_all(root.with_extension("old-home"));
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // The home this operation opened holds no key ...
    std::fs::write(ssh.join("authorized_keys"), "ssh-ed25519 AAAAC3Nzakeep\n").unwrap();
    // ... and the replacement holds the key that is to be absent.
    let replaced = root.clone();
    let moved = root.with_extension("old-home");
    let moved_back = moved.clone();
    BEFORE_PUBLISH
        .lock()
        .unwrap()
        .push(std::sync::Arc::new(move |home: &std::path::Path| {
            if home != replaced {
                return;
            }
            std::fs::rename(&replaced, &moved).unwrap();
            std::fs::create_dir(&replaced).unwrap();
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
    assert!(error.contains("home changed"), "{error}");
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&moved_back).unwrap();
}

#[test]
fn a_replaced_home_stops_the_removal() {
    let _arming = ARMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = scratch("home-detached");
    // A failed run leaves the renamed home behind: clear it with the
    // scratch directory so the swap below always starts empty.
    let _ = std::fs::remove_dir_all(root.with_extension("old-home"));
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    std::fs::write(
        ssh.join("authorized_keys"),
        "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\n",
    )
    .unwrap();
    // The home is replaced while the removal is between its check and
    // its report: the edit lands in the detached tree, and the removal
    // fails instead of reporting a change it did not make.
    let replaced = root.clone();
    let moved = root.with_extension("old-home");
    let moved_back = moved.clone();
    BEFORE_PUBLISH
        .lock()
        .unwrap()
        .push(std::sync::Arc::new(move |home: &std::path::Path| {
            if home != replaced {
                return;
            }
            std::fs::rename(&replaced, &moved).unwrap();
            std::fs::create_dir(&replaced).unwrap();
            std::fs::create_dir(replaced.join(".ssh")).unwrap();
            std::fs::write(
                replaced.join(".ssh/authorized_keys"),
                "ssh-ed25519 AAAAC3Nzakeep\n",
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
    assert!(error.contains("home changed"), "{error}");
    // The visible replacement is untouched: it never held the key, and
    // the removal did not pretend otherwise.
    assert_eq!(
        std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
        "ssh-ed25519 AAAAC3Nzakeep\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&moved_back).unwrap();
}

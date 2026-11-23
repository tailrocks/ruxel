//! The atomic replacement of a file: what it keeps, and what an attacker
//! in the parent directory cannot do to it.

use super::super::{write_atomic, write_atomic_with};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[test]
fn atomic_write_replaces_symlink_not_its_target() {
    let root = std::env::temp_dir().join(format!("ruxel-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let victim = root.join("victim");
    let dest = root.join("dest");
    std::fs::write(&victim, "safe").unwrap();
    symlink(&victim, &dest).unwrap();
    write_atomic(&dest, b"replacement").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "replacement");
    assert!(
        !std::fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Beyond the platform's raw mode width the conversion reports the range,
/// which only the narrower platforms can reach. The write refuses before any
/// file exists, so nothing is published under a truncated mode.
#[cfg(not(target_os = "linux"))]
#[test]
fn atomic_write_rejects_modes_exceeding_the_platform_range() {
    let root = std::env::temp_dir().join(format!("ruxel-mode-range-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dest = root.join("dest");
    let error = write_atomic_with(&dest, b"content", Some(1 << 20), (None, None)).unwrap_err();
    assert!(
        error.contains("file mode exceeds platform range"),
        "{error}"
    );
    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        0,
        "the refused write left a file behind"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A second thread keeps swapping the temporary pathname for a symlink to
/// the victim while a replacement is written: ownership and mode must reach
/// the file that was opened, so a swapped pathname cannot redirect the
/// privileged calls onto the victim.
#[cfg(unix)]
#[test]
fn swapping_the_temporary_path_for_a_symlink_cannot_redirect_ownership() {
    let root = std::env::temp_dir().join(format!("ruxel-atomic-swap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let victim = root.join("victim");
    std::fs::write(&victim, "victim").unwrap();
    // A mode no atomic write would request, so a redirected chmod shows.
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o640)).unwrap();
    let uid = std::fs::metadata(&victim).unwrap().uid();
    let gid = std::fs::metadata(&victim).unwrap().gid();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let racer_root = root.clone();
    let racer_victim = victim.clone();
    let racer_stop = stop.clone();
    let racer = std::thread::spawn(move || {
        while !racer_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let Ok(entries) = std::fs::read_dir(&racer_root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') || !name.ends_with(".tmp") {
                    continue;
                }
                let path = entry.path();
                if std::fs::symlink_metadata(&path).is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
                let _ = symlink(&racer_victim, &path);
            }
        }
    });

    // A large body keeps the write running, so the swap lands while the
    // replacement is open: the ownership and mode calls follow. A swap that
    // races the publish itself fails the write; that failure is the racer's,
    // not the function's, so it retries.
    let buffer = vec![7u8; 64 * 1024 * 1024];
    let dest = root.join("dest");
    let mut published = 0;
    for _ in 0..200 {
        if write_atomic_with(&dest, &buffer, Some(0o600), (Some(uid), Some(gid))).is_ok() {
            published += 1;
            if published == 3 {
                break;
            }
        }
    }
    assert_eq!(published, 3, "the racer starved the writes");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    racer.join().unwrap();

    let mode = std::fs::metadata(&victim).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o640, "the victim's mode was redirected");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
    let _ = std::fs::remove_dir_all(&root);
}

/// A second thread keeps swapping the temporary pathname for a symlink to
/// an attacker file while a replacement is written: the publication must
/// carry the bytes of the file that was opened. The attacker file must
/// never reach the destination, and the destination must never be the
/// attacker's symlink.
///
/// The publication is bound to the opened descriptor on Linux, where the
/// agent ships. The other platforms publish the temporary pathname, so the
/// race there is the platform's own.
#[cfg(target_os = "linux")]
#[test]
fn swapping_the_temporary_path_cannot_publish_the_attacker_file() {
    let root = std::env::temp_dir().join(format!("ruxel-atomic-publish-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let attacker = root.join("attacker");
    std::fs::write(&attacker, "attacker").unwrap();
    let dest = root.join("dest");

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let racer_root = root.clone();
    let racer_attacker = attacker.clone();
    let racer_stop = stop.clone();
    let racer = std::thread::spawn(move || {
        while !racer_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let Ok(entries) = std::fs::read_dir(&racer_root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') || !name.ends_with(".tmp") {
                    continue;
                }
                let path = entry.path();
                if std::fs::symlink_metadata(&path).is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
                let _ = symlink(&racer_attacker, &path);
            }
        }
    });

    // A large body keeps the write running, so the swap lands while the
    // replacement is open. A swap that races the publication itself may
    // fail the write; that failure is the racer's, not the function's, so
    // the write is retried.
    let buffer = vec![7u8; 64 * 1024 * 1024];
    let mut published = 0;
    for _ in 0..200 {
        if write_atomic_with(&dest, &buffer, Some(0o600), (None, None)).is_ok() {
            published += 1;
            // The destination holds the bytes that were written, not the
            // attacker's file and not the attacker's symlink.
            assert!(
                !std::fs::symlink_metadata(&dest)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "the attacker symlink was published"
            );
            let content = std::fs::read(&dest).unwrap();
            assert_eq!(content.len(), buffer.len(), "the published file is shorter");
            assert!(
                content.iter().all(|byte| *byte == 7),
                "the published file is not the written one"
            );
            assert_eq!(
                std::fs::read_to_string(&attacker).unwrap(),
                "attacker",
                "the attacker file was published"
            );
            if published == 3 {
                break;
            }
        }
    }
    assert_eq!(published, 3, "the racer starved the writes");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    racer.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// The destination's parent is opened once, before the write begins: an
/// attacker who renames the parent directory and puts a symlink to a
/// directory of their own in its place cannot redirect the write. The
/// write lands in the directory it opened, and the attacker's directory
/// stays empty.
#[cfg(target_os = "linux")]
#[test]
fn renaming_the_parent_directory_cannot_redirect_the_write() {
    use std::sync::atomic::Ordering;

    let root = std::env::temp_dir().join(format!("ruxel-atomic-parent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let parent = root.join("parent");
    let attacker = root.join("attacker");
    let moved = root.join("moved");
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(&attacker).unwrap();
    let dest = parent.join("dest");

    // The write pauses after the parent is opened; the test moves the
    // parent under it.
    super::super::atomic::pause::PARENT.store(2_000, Ordering::Relaxed);
    let writer = std::thread::spawn(move || {
        let result = write_atomic(&dest, b"written");
        super::super::atomic::pause::PARENT.store(0, Ordering::Relaxed);
        result
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The parent is renamed away, and a symlink to the attacker's
    // directory takes its place.
    std::fs::rename(&parent, &moved).unwrap();
    symlink(&attacker, &parent).unwrap();
    writer.join().unwrap().unwrap();

    // The write landed in the directory it opened, not in the attacker's.
    assert!(
        !attacker.join("dest").exists(),
        "the write was redirected into the attacker's directory"
    );
    assert_eq!(
        std::fs::read_to_string(moved.join("dest")).unwrap(),
        "written"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A destination that becomes a directory between the check and the
/// exchange is not left stranded under the staging name: the write puts
/// it back where it was and fails loudly.
#[cfg(target_os = "linux")]
#[test]
fn a_destination_that_becomes_a_directory_fails_the_write_loudly() {
    use std::sync::atomic::Ordering;

    // The exchange needs a filesystem that has it: tmpfs does, and the
    // temporary directory of a container does not always.
    let base = if Path::new("/dev/shm").is_dir() {
        PathBuf::from("/dev/shm")
    } else {
        std::env::temp_dir()
    };
    let root = base.join(format!("ruxel-atomic-exchange-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dest = root.join("dest");
    std::fs::write(&dest, "old").unwrap();

    // The write pauses after it checked the destination; the test turns
    // the destination into a directory under it.
    super::super::atomic::pause::EXCHANGE.store(2_000, Ordering::Relaxed);
    let written = dest.clone();
    let writer = std::thread::spawn(move || {
        let result = write_atomic(&written, b"written");
        super::super::atomic::pause::EXCHANGE.store(0, Ordering::Relaxed);
        result
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    std::fs::remove_file(&dest).unwrap();
    std::fs::create_dir(&dest).unwrap();
    let error = writer.join().unwrap().unwrap_err();
    assert!(error.contains("directory"), "{error}");
    // The destination is the directory it was: the write put it back.
    assert!(dest.is_dir(), "the destination is not a directory");
    let _ = std::fs::remove_dir_all(root);
}

/// The payload of a write that fails loudly must not survive the failure:
/// the staging directory held the newly written bytes, and it must be
/// removed with them rather than left behind under a private name.
#[cfg(target_os = "linux")]
#[test]
fn the_staging_payload_is_removed_when_the_replacement_fails() {
    use std::sync::atomic::Ordering;

    // The exchange needs a filesystem that has it: tmpfs does, and the
    // temporary directory of a container does not always.
    let base = if Path::new("/dev/shm").is_dir() {
        PathBuf::from("/dev/shm")
    } else {
        std::env::temp_dir()
    };
    let root = base.join(format!("ruxel-atomic-staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dest = root.join("dest");
    std::fs::write(&dest, "old").unwrap();

    // The write pauses after it checked the destination; the test turns
    // the destination into a directory under it.
    super::super::atomic::pause::EXCHANGE.store(2_000, Ordering::Relaxed);
    let written = dest.clone();
    let writer = std::thread::spawn(move || {
        let result = write_atomic(&written, b"synthetic-secret-payload");
        super::super::atomic::pause::EXCHANGE.store(0, Ordering::Relaxed);
        result
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    std::fs::remove_file(&dest).unwrap();
    std::fs::create_dir(&dest).unwrap();
    let error = writer.join().unwrap().unwrap_err();
    assert!(error.contains("directory"), "{error}");
    assert!(dest.is_dir(), "the destination is not a directory");
    // Only the destination remains: no staging directory and no temporary
    // name of the write survives the failure, so the payload is gone too.
    for entry in std::fs::read_dir(&root).unwrap() {
        let name = entry.unwrap().file_name();
        assert_eq!(
            name,
            std::ffi::OsStr::new("dest"),
            "the failed write left {name:?} behind"
        );
    }
    assert_eq!(
        std::fs::read_dir(&dest).unwrap().count(),
        0,
        "the destination directory is not the empty one the test made"
    );
    let _ = std::fs::remove_dir_all(root);
}

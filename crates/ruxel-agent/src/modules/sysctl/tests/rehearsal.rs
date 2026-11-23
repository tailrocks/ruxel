//! The rehearsal's staging primitives: the absent target rehearses
//! from the probe handle's own inode instead of a second full copy,
//! and the exchange rehearsal takes the atomic publish's own fallback
//! where the filesystem does not support the exchange.

use super::super::{exchange_failed_unsupported, rehearse_absent};
use std::os::unix::fs::MetadataExt;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-rehearsal-{name}-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    dir
}

#[test]
fn the_absent_target_rehearses_from_the_probe_inode() {
    // Thread 350: the absent-target branch wrote the content into the
    // probe handle and then wrote a second full copy with
    // `stage_plain`, so a quota fitting one copy but not two failed the
    // rehearsal where the publish — one inode plus rename — succeeds.
    // The rehearsal must link the already-staged bytes instead.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = scratch("single-copy");
    let file = dir.join("50-ruxel.conf");
    let text = file.to_str().expect("path is text").to_owned();
    let scratch_path = dir.join("probe");
    let mut staged = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&scratch_path)
        .expect("operation succeeds");
    staged
        .write_all(b"net.test=1\n")
        .expect("operation succeeds");
    let first = dir.join("first");
    let second = dir.join("second");
    rehearse_absent(&text, &scratch_path, &staged, &first, &second)
        .expect("an absent target must rehearse from the probe");
    drop(staged);
    let probe_meta = std::fs::metadata(&scratch_path).expect("operation succeeds");
    let first_meta = std::fs::metadata(&first).expect("operation succeeds");
    assert_eq!(
        (probe_meta.dev(), probe_meta.ino()),
        (first_meta.dev(), first_meta.ino()),
        "the rehearsal stages one inode, not a second full copy"
    );
    assert_eq!(
        std::fs::read(&first).expect("operation succeeds"),
        b"net.test=1\n",
        "the linked bytes are the publish's bytes"
    );
    assert_eq!(
        std::fs::read(&second).expect("operation succeeds"),
        b"net.test=1\n",
        "the exchange rehearsal swaps the staged bytes into place for the caller to retire"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_exchange_fallback_mirrors_the_publish() {
    // Thread 349: the rehearsal mapped RENAME_EXCHANGE errors directly
    // while the atomic publish falls back to unlink-plus-rename on
    // INVAL, NOSYS and NOTSUP — refusing writes the publish performs.
    // The rehearsal must take the fallback on exactly that errno set.
    use rustix::io::Errno;

    for errno in [Errno::INVAL, Errno::NOSYS, Errno::NOTSUP] {
        assert!(
            exchange_failed_unsupported(errno),
            "{errno} falls back in the publish, so the rehearsal must too"
        );
    }
    for errno in [Errno::PERM, Errno::ACCESS, Errno::NOENT, Errno::EXIST] {
        assert!(
            !exchange_failed_unsupported(errno),
            "{errno} refuses the publish, so the rehearsal must refuse it too"
        );
    }
}

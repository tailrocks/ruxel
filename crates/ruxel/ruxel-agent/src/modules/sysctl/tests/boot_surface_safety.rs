//! Boot surface safety: entries the scanner must never open and
//! directories it must refuse instead of reading as empty.

use super::super::boot;
use super::boot_surface::Scratch;

#[test]
fn a_fifo_entry_never_blocks_the_fingerprint() {
    // A selected `*.conf` entry may be a FIFO with no writer: opening
    // it blocks indefinitely, so the fingerprint classifies entries
    // the way the override probe does and never opens one. The FIFO
    // holds the name but contributes no assignments — the surface
    // over it is the empty surface's, and the boot value stands.
    let scratch = Scratch::new("fifo");
    let dir = scratch.dir("etc");
    let fifo = dir.join("99-fifo.conf");
    if !std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|status| status.success())
    {
        // No FIFO support on this host: nothing to prove.
        return;
    }
    assert_eq!(
        boot::boot_bytes(&fifo).expect("the surface classifies"),
        None,
        "a fifo loads as nothing"
    );
    let files = vec![fifo.clone()];
    assert_eq!(
        boot::fingerprint_of(&files).expect("a fifo entry must not refuse the surface"),
        boot::fingerprint_of(&[]).expect("an empty surface fingerprints"),
        "a fifo contributes no assignments"
    );
    let host = vec![(fifo, Vec::new())];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "0",
            &host,
            &[],
        ),
        None,
        "the fifo assignment must not hand the boot value away"
    );
}

#[test]
fn an_unreadable_boot_directory_refuses_instead_of_reading_as_empty() {
    // `read_dir` fails where the directory cannot be read — a path
    // that is a file (`ENOTDIR`), permissions, I/O — and only a
    // missing directory reads as empty: the boot loader reads the
    // directory while the agent cannot, so its later-loading
    // assignments are unknown and the surface refuses instead of
    // hiding them from the override preflight and the fingerprint.
    let scratch = Scratch::new("unreadable");
    // A missing directory holds no names and reads as empty.
    let missing = scratch.0.join("missing");
    let text = missing.to_str().expect("path is text").to_owned();
    let dirs = [(text.as_str(), 1u8)];
    let files = boot::system_files_in(&dirs).expect("a missing directory reads as empty");
    assert!(
        files.iter().all(|file| !file.starts_with(&scratch.0)),
        "a missing directory contributes no names: {files:?}"
    );
    // A path that cannot be listed refuses the surface.
    let blocker = scratch.0.join("blocker");
    std::fs::write(&blocker, "not a directory\n").expect("operation succeeds");
    let text = blocker.to_str().expect("path is text").to_owned();
    let dirs = [(text.as_str(), 1u8)];
    let error = boot::system_files_in(&dirs).expect_err("an unreadable directory must refuse");
    assert!(
        error.contains("cannot read the boot directory"),
        "the refusal must name the directory: {error}"
    );
}

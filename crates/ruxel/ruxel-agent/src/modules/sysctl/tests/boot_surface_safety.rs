//! Boot surface safety: entries the scanner must never open and
//! directories it must refuse instead of reading as empty.

use super::super::boot;
use super::boot_surface::Scratch;

#[test]
fn a_fifo_entry_never_blocks_the_fingerprint() {
    // A selected `*.conf` entry may be a FIFO with no writer: opening
    // it blocks indefinitely, so the surface classifies the entry by
    // type and never opens one — and a FIFO refuses instead of loading
    // as nothing, the way procps blocks on it instead of skipping it.
    // The refusal travels the fail-safe path: the preflight refuses
    // and the fingerprint misses, so no cache is served over it.
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
    let error = boot::boot_bytes(&fifo).expect_err("a FIFO must refuse, not load as empty");
    assert!(error.contains("FIFO"), "{error}");
    assert!(
        boot::fingerprint_of(std::slice::from_ref(&fifo)).is_none(),
        "a FIFO invalidates the boot fingerprint"
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

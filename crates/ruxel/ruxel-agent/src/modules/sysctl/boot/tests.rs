//! The name is what the boot selects: every entry holds its
//! basename even where it contributes no bytes.

use super::scratch::{Scratch, dirs_of};
use super::{boot_bytes, boot_override, scan_error, system_files_in};
use std::path::PathBuf;
#[test]
fn a_link_to_a_directory_holds_the_name_at_boot() {
    // procps records the entry's basename before it tries to open
    // it: a higher-precedence `*.conf` link to an existing
    // directory holds the name although `is_file`, following the
    // link, answers false — the old scan dropped the name and
    // admitted the lower-precedence copy, whose assignments procps
    // never loads. The selection is the name alone; the unreadable
    // entry loads as nothing.
    let scratch = Scratch::new("dir-link");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let target = scratch.dir("target");
    std::os::unix::fs::symlink(target, etc.join("99-vendor.conf"))
        .expect("the link points at a directory");
    std::fs::write(usr.join("99-vendor.conf"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs).expect("the scratch directories read");
    assert!(
        files.iter().any(|file| file == &etc.join("99-vendor.conf")),
        "the link holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file.starts_with(&usr)),
        "the shadowed copy must not load: {files:?}"
    );
    // The selected entry contributes no assignments, so the managed
    // value stands: only the lower copy the boot never loads
    // assigns another value.
    let host: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|file| {
            (
                file.clone(),
                boot_bytes(file)
                    .expect("the surface classifies")
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        boot_override(
            "/run/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "0",
            &host,
            &super::super::SysctlManaged::default()
        ),
        None,
        "the shadowed assignment must not hand the boot value away"
    );
}

#[test]
fn a_directory_holds_the_name_at_boot() {
    // A real subdirectory named `*.conf` is no openable file
    // either, and procps selects its basename all the same: the
    // lower-precedence copy of the name never loads and weighs
    // nothing on the boot value.
    let scratch = Scratch::new("dir-entry");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    std::fs::create_dir_all(etc.join("99-vendor.conf")).expect("operation succeeds");
    std::fs::write(usr.join("99-vendor.conf"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs).expect("the scratch directories read");
    assert!(
        files.iter().any(|file| file == &etc.join("99-vendor.conf")),
        "the directory holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file.starts_with(&usr)),
        "the shadowed copy must not load: {files:?}"
    );
    let host: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|file| {
            (
                file.clone(),
                boot_bytes(file)
                    .expect("the surface classifies")
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        boot_override(
            "/run/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "0",
            &host,
            &super::super::SysctlManaged::default()
        ),
        None,
        "the shadowed assignment must not hand the boot value away"
    );
}

#[test]
fn a_null_mask_still_loads_as_nothing() {
    // The standard mask is a symlink to the null device: it holds
    // the name and contributes no assignments, whatever special type
    // the target carries.
    let scratch = Scratch::new("mask");
    let mask = scratch.0.join("50-mask.conf");
    std::os::unix::fs::symlink("/dev/null", &mask).expect("operation succeeds");
    assert_eq!(
        boot_bytes(&mask).expect("the null mask loads"),
        None,
        "the null mask holds the name and contributes nothing"
    );
}

#[test]
fn a_per_entry_scan_error_refuses_with_the_directory_named() {
    // No portable fixture makes a healthy directory's iterator yield
    // an I/O error: the failure needs a filesystem fault mid-scan —
    // a directory deleted or replaced under the open handle, a remote
    // filesystem failing between entries — so the mapping the
    // iterator errors travel through is pinned directly. The refusal
    // names the directory, the way the `read_dir` refusal does,
    // instead of dropping the entry and hiding a later-loading
    // assignment from the override probe and the fingerprint.
    let error = scan_error("/etc/sysctl.d", &std::io::Error::other("stale file handle"));
    assert!(
        error.contains("cannot read the boot directory /etc/sysctl.d"),
        "the refusal must name the directory: {error}"
    );
    assert!(
        error.contains("stale file handle"),
        "the refusal must carry the failure: {error}"
    );
}

#[test]
fn a_dot_conf_dotfile_is_a_boot_file() {
    // Thread 335: `Path::extension` reports no extension for a
    // basename consisting entirely of `.conf`, yet procps loads that
    // dotfile — `sysctl --dry-run --system` prints `* Applying
    // /run/sysctl.d/.conf ...`. The scan matches the raw basename's
    // `.conf` suffix, or an unmanaged `.conf` that sorts after an
    // earlier managed file overrides its key invisibly.
    let scratch = Scratch::new("dot-conf");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    std::fs::write(etc.join(".conf"), "net.ipv4.ip_forward=1\n").expect("operation succeeds");
    std::fs::write(etc.join("README"), "not a sysctl file\n").expect("operation succeeds");
    std::fs::write(etc.join("stray.conf.bak"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs).expect("the scratch directories read");
    assert!(
        files.iter().any(|file| file == &etc.join(".conf")),
        "the .conf dotfile loads at boot: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &etc.join("README")),
        "a bare name stays out: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &etc.join("stray.conf.bak")),
        "a backup suffix stays out: {files:?}"
    );
}

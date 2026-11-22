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
            &[]
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
            &[]
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
fn a_dangling_relative_alias_is_no_shadow() {
    // Thread 297: a higher-precedence relative link to a managed file
    // that does not exist yet compares literally and mismatches —
    // normalizing the lexical `..` components admits the genuine
    // self-alias the publish creates, instead of refusing a valid
    // bootstrap layout as a shadow.
    let scratch = Scratch::new("alias");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let candidate = etc.join("x.conf");
    std::os::unix::fs::symlink("../usr/x.conf", &candidate).expect("operation succeeds");
    let file = usr.join("x.conf");
    assert!(
        !file.exists(),
        "both sides dangle: the managed file is not published yet"
    );
    assert!(
        super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "the dangling relative alias is the managed file itself"
    );
    // A dangling link anywhere else is still a shadow.
    let elsewhere = etc.join("y.conf");
    std::os::unix::fs::symlink("../usr/z.conf", &elsewhere).expect("operation succeeds");
    assert!(
        !super::shadow::is_self_alias(
            elsewhere.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a dangling link elsewhere still shadows"
    );
}

#[test]
fn a_symlink_beneath_a_dangling_alias_is_no_self_alias() {
    // Thread 297: a `..` past a link resolves from the link's
    // target, not from the spelling — collapsing it lexically across
    // the link names a file the entry never reaches. Here the
    // candidate's target crosses the `link` component, which points at
    // a live directory elsewhere: lexically the alias names the
    // managed file, but the loader resolves it past the link's target
    // instead, so the dangling pair must refuse rather than exempt
    // the shadow.
    let scratch = Scratch::new("alias-symlink");
    let elsewhere = Scratch::new("alias-elsewhere");
    let sub = scratch.dir("sub");
    let candidate = sub.join("x.conf");
    std::os::unix::fs::symlink("../link/../managed.conf", &candidate).expect("operation succeeds");
    std::os::unix::fs::symlink(elsewhere.0.clone(), sub.join("link")).expect("operation succeeds");
    let file = scratch.0.join("managed.conf");
    assert!(
        !candidate.exists() && !file.exists(),
        "both sides dangle: the link target and the managed file are absent"
    );
    assert!(
        !super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a dangling alias crossing a symlink is no self-alias"
    );
    // The same shape with no symlink beneath it stays a genuine
    // self-alias: the refusal above is the link, not the layout.
    let plain = sub.join("y.conf");
    std::os::unix::fs::symlink("../managed.conf", &plain).expect("operation succeeds");
    assert!(
        super::shadow::is_self_alias(
            plain.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "the link-free twin is still the managed file itself"
    );
}

#[test]
fn a_looping_alias_never_matches_lexically() {
    // Thread 297: only a missing file counts as dangling — a link
    // loop fails with a filesystem-loop error, not `NotFound`, so
    // even a lexically equal spelling refuses instead of exempting
    // the shadow.
    let scratch = Scratch::new("alias-loop");
    let sub = scratch.dir("sub");
    let candidate = sub.join("x.conf");
    std::os::unix::fs::symlink("x.conf", &candidate).expect("operation succeeds");
    let file = sub.join(".").join("..").join("sub").join("x.conf");
    assert!(
        !candidate.exists() && !file.exists(),
        "both sides fail to resolve: the loop never reaches a file"
    );
    assert!(
        !super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a looping alias is no self-alias whatever the spelling"
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

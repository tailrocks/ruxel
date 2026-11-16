//! The names the boot surface holds: a name is held by the entry the
//! directory lists, not by what the entry resolves to, and the boot
//! files load in the order of the bytes their names hold.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::super::boot;
use super::Scratch;

#[test]
fn a_broken_link_in_a_higher_directory_holds_the_name() {
    // procps reads the directories for the names they hold
    // (`PreloadSystem`), not for what the entries resolve to: the
    // broken link holds the name like any other entry — procps selects
    // it, reports that it cannot open it, and never loads the same
    // name from the lower-precedence directory. The copy below it is
    // therefore not part of the boot surface at all.
    let scratch = Scratch::new("broken-name");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let broken = etc.join("90-vendor.conf");
    std::os::unix::fs::symlink(scratch.0.join("absent"), &broken).expect("the link is created");
    let vendor = usr.join("90-vendor.conf");
    std::fs::write(&vendor, "net.ipv4.ip_forward=1\n").expect("the file is written");
    let etc_dir = format!("{}/", etc.display());
    let usr_dir = format!("{}/", usr.display());
    let dirs = [(etc_dir.as_str(), 1u8), (usr_dir.as_str(), 0u8)];
    let files = boot::system_files_in(&dirs);
    assert!(
        !files.iter().any(|file| file == &vendor),
        "the copy below the broken link never loads: {files:?}"
    );
}

#[test]
fn a_boot_file_whose_name_is_not_text_loads_after_the_managed_one() {
    // procps sorts the boot files by the bytes of their names
    // (`PreloadSystem`), so a file whose name holds the byte 0xff loads
    // after one whose name ends in the emoji: the lossy spelling of
    // that byte (U+FFFD) would sort it before, and the assignment that
    // overrides the managed value would be missed.
    let managed = "/etc/sysctl.d/90-ruxel\u{1f600}.conf";
    let raw = OsString::from_vec(b"/etc/sysctl.d/90-ruxel\xff.conf".to_vec());
    let host = vec![
        (managed.to_string(), b"net.ipv4.ip_forward=1\n".to_vec()),
        (
            raw.to_string_lossy().into_owned(),
            b"net.ipv4.ip_forward=0\n".to_vec(),
        ),
    ];
    assert_eq!(
        boot::boot_override(managed, "net.ipv4.ip_forward", "1", &host, &[]),
        Some((raw.to_string_lossy().into_owned(), "0".to_string()))
    );
}

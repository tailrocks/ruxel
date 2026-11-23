//! The names of the boot files: a `*.conf` basename need not be valid
//! text, and procps sorts the names as their raw bytes.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use super::super::boot;

/// A path whose base name holds a byte that is not text.
fn path_with_a_byte_that_is_not_text(name: &[u8]) -> PathBuf {
    let mut whole = b"/etc/sysctl.d/".to_vec();
    whole.extend_from_slice(name);
    whole.extend_from_slice(b".conf");
    PathBuf::from(OsString::from_vec(whole))
}

#[test]
fn a_name_that_is_not_text_loads_after_the_emoji() {
    // procps sorts the base names as their raw bytes: the emoji
    // (0xf0 0x9f 0x98 0x80) sorts before the name that holds a byte
    // that is not text (0xff), so the file that is not text loads
    // after it and its assignment overrides the managed entry. A lossy
    // spelling compares the other way round — the replacement
    // character (0xef 0xbf 0xbd) sorts before the emoji — and the
    // override is missed.
    let managed = PathBuf::from("/etc/sysctl.d/\u{1f600}.conf");
    let later = path_with_a_byte_that_is_not_text(b"\xff");
    let host = vec![
        (managed.clone(), b"net.ipv4.ip_forward=1\n".to_vec()),
        (later.clone(), b"net.ipv4.ip_forward=0\n".to_vec()),
    ];
    assert_eq!(
        boot::boot_override(
            &managed.to_string_lossy(),
            "net.ipv4.ip_forward",
            "1",
            &host,
            &super::super::SysctlManaged::default(),
        ),
        Some((later.to_string_lossy().into_owned(), "0".to_string())),
        "the file that loads after the managed one overrides it"
    );
}

#[test]
fn a_name_that_is_not_text_loads_before_the_emoji() {
    // The other way round: the name that holds a byte that is not
    // text sorts before the emoji, so it loads before the managed file
    // and its assignment is overridden by the managed entry.
    let managed = PathBuf::from("/etc/sysctl.d/\u{1f600}.conf");
    let earlier = path_with_a_byte_that_is_not_text(b"\x01");
    let host = vec![
        (managed.clone(), b"net.ipv4.ip_forward=1\n".to_vec()),
        (earlier.clone(), b"net.ipv4.ip_forward=0\n".to_vec()),
    ];
    assert_eq!(
        boot::boot_override(
            &managed.to_string_lossy(),
            "net.ipv4.ip_forward",
            "1",
            &host,
            &super::super::SysctlManaged::default(),
        ),
        None,
        "the file that loads before the managed one overrides nothing"
    );
}

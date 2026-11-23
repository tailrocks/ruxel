//! The syntax-blank trim: space, tab, vertical tab, form feed — the
//! `isspace` set procps strips around a value — and nothing else.

use std::path::PathBuf;

use super::super::boot;

#[test]
fn vertical_tab_trims_like_the_other_syntax_blanks() {
    // procps `lstrip`/`rstrip` read `isspace`, which holds the
    // vertical tab: a VT-padded value is one kernel value with its bare
    // spelling, the way space-, tab- and FF-padded ones are.
    assert_eq!(super::super::trimmed_value("\u{B}1\u{B}"), "1");
    assert_eq!(super::super::trimmed_value(" \t\u{B}1\u{C}\t "), "1");
    assert_eq!(super::super::trim(b"\x0B 1 \x0C"), b"1");
}

#[test]
fn the_trim_keeps_whitespace_outside_the_syntax_set() {
    // A no-break space is value content, not a syntax blank: the trim
    // must not reach it, whatever side it pads.
    assert_eq!(super::super::trimmed_value("nb\u{A0}"), "nb\u{A0}");
    assert_eq!(super::super::trimmed_value("\u{A0}nb"), "\u{A0}nb");
    assert_eq!(
        super::super::trim("\u{A0}1".as_bytes()),
        "\u{A0}1".as_bytes()
    );
}

#[test]
fn a_vertically_padded_file_entry_is_the_managed_value() {
    // The rewrite compares trimmed values: a file that pads the
    // managed value with vertical tabs holds no drift, so the apply
    // publishes nothing.
    let (content, changed) =
        super::super::rewrite_file("net.test=\u{B}1\u{B}\n".as_bytes(), "net.test", "1");
    assert!(!changed);
    assert_eq!(content, "net.test=\u{B}1\u{B}\n".as_bytes());
    // Padding another value is still drift, rewritten to the managed
    // spelling.
    let (content, changed) =
        super::super::rewrite_file("net.test=\u{B}2\u{B}\n".as_bytes(), "net.test", "1");
    assert!(changed);
    assert_eq!(content, b"net.test=1\n".to_vec());
}

#[test]
fn the_live_value_matches_across_vertical_tabs() {
    // The live value a VT-padded read holds is the bare value the
    // workflow asks for: no `sysctl -w` runs on an idempotent apply.
    assert!(super::super::live_matches("kernel.hostname", "\u{B}1", "1"));
    assert!(super::super::live_matches("kernel.hostname", "1\u{B}", "1"));
    assert!(super::super::live_matches(
        "kernel.hostname",
        "\u{B}1\u{B}",
        "1"
    ));
    // A padded different value is still reconciled.
    assert!(!super::super::live_matches(
        "kernel.hostname",
        "\u{B}2\u{B}",
        "1"
    ));
}

#[test]
fn a_vertically_padded_key_is_still_the_named_entry() {
    // The line trim covers the key side too: procps strips the same
    // blanks around the key, so a VT-padded key names its entry.
    assert_eq!(
        boot::named_keys("net.ipv4.ip_forward=\u{B}1\u{B}\n".as_bytes()),
        ["net/ipv4/ip_forward".to_string()]
    );
}

#[test]
fn a_vertically_padded_later_file_holds_the_managed_value() {
    // The boot comparison trims the same way: a later-loading file
    // that pads the managed value with vertical tabs assigns the
    // managed value itself, so there is no override to refuse.
    let host: Vec<(PathBuf, Vec<u8>)> = [
        ("/etc/sysctl.d/10-ruxel.conf", "net.ipv4.ip_forward=1\n"),
        (
            "/etc/sysctl.d/90-later.conf",
            "net.ipv4.ip_forward=\u{B}1\u{B}\n",
        ),
    ]
    .iter()
    .map(|(path, content)| (PathBuf::from(path), content.as_bytes().to_vec()))
    .collect();
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &super::super::SysctlManaged::default()
        ),
        None,
        "the VT-padded later assignment is the managed value"
    );
}

#[test]
fn a_crlf_later_file_holds_the_managed_value() {
    // The carriage return is the line ending, not the value: a CRLF
    // host file's entry compares like its LF spelling, so the later
    // file that carries the managed value refuses nothing.
    let host: Vec<(PathBuf, Vec<u8>)> =
        [("/etc/sysctl.d/90-later.conf", "net.ipv4.ip_forward=1\r\n")]
            .iter()
            .map(|(path, content)| (PathBuf::from(path), content.as_bytes().to_vec()))
            .collect();
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &super::super::SysctlManaged::default()
        ),
        None,
        "the CRLF later assignment is the managed value"
    );
}

//! The wildcards of the boot sequence: a pattern expands to the keys it
//! covers, and a key the sequence names outright is kept out of that
//! expansion.

use super::super::boot;

/// The later-loading file that overrides the managed entry, if any.
fn override_of(
    file: &str,
    name: &str,
    value: &str,
    host: &[(&str, &str)],
) -> Option<(String, String)> {
    let host: Vec<(String, Vec<u8>)> = host
        .iter()
        .map(|(path, content)| ((*path).to_string(), content.as_bytes().to_vec()))
        .collect();
    boot::boot_override(file, name, value, &host, &[])
}

#[test]
fn a_wildcard_reaches_no_key_the_managed_file_assigns_by_name() {
    // procps keeps the keys a setting names outright out of the
    // expansion of a pattern: the managed file's own entry is an
    // assignment by name, so a later `net.ipv4.conf.*.rp_filter` never
    // reaches `net.ipv4.conf.all.rp_filter` and hands no boot value
    // away from it — while a key the sequence never names is still
    // reported (see `boot_surface`'s
    // `an_explicit_assignment_is_excluded_from_a_later_wildcard`).
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &[("/etc/sysctl.conf", "net.ipv4.conf.*.rp_filter=0\n")],
        ),
        None
    );
}

#[test]
fn a_wildcard_key_that_covers_another_key_is_not_this_entry() {
    // The wildcard covers the interfaces of another parameter: it does
    // not assign the managed key.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[("/etc/sysctl.conf", "net.ipv4.conf.*.rp_filter=0\n")],
        ),
        None
    );
}

#[test]
fn a_pending_named_assignment_in_an_existing_file_excludes_the_wildcard() {
    // The managed file exists but holds no line for the managed key
    // yet: the entry under management is an assignment by name that
    // `rewrite_file` is about to write, and procps keeps a key a
    // setting names outright out of the expansion of a pattern — so
    // the later `net.ipv4.conf.*.rp_filter` never reaches
    // `net.ipv4.conf.all.rp_filter`, and the boot value stays the
    // managed one. The file's absence is not what makes the
    // difference: the assignment is pending either way.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &[
                ("/etc/sysctl.d/90-ruxel.conf", "# managed by ruxel\n"),
                ("/etc/sysctl.conf", "net.ipv4.conf.*.rp_filter=0\n"),
            ],
        ),
        None
    );
}

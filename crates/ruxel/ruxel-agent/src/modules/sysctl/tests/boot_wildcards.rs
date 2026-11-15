//! The wildcards of the boot sequence: a pattern expands to the keys it
//! covers, and a key the sequence names outright is kept out of that
//! expansion.

use std::path::PathBuf;

use super::super::boot;

/// The later-loading file that overrides the managed entry, if any.
fn override_of(
    file: &str,
    name: &str,
    value: &str,
    host: &[(&str, &str)],
) -> Option<(String, String)> {
    let host: Vec<(PathBuf, Vec<u8>)> = host
        .iter()
        .map(|(path, content)| (PathBuf::from(*path), content.as_bytes().to_vec()))
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
fn a_pending_named_assignment_beside_another_key_excludes_the_wildcard() {
    // The managed file exists and names another key: the entry under
    // management is still the assignment by name that `rewrite_file`
    // is about to write, and the file's own other assignments say
    // nothing about it — procps keeps the key a setting names outright
    // out of the expansion of a pattern, so the later
    // `net.ipv4.conf.*.rp_filter` never reaches
    // `net.ipv4.conf.all.rp_filter` and hands no boot value away.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &[
                ("/etc/sysctl.d/90-ruxel.conf", "net.ipv4.ip_forward=1\n"),
                ("/etc/sysctl.conf", "net.ipv4.conf.*.rp_filter=0\n"),
            ],
        ),
        None
    );
}

#[test]
fn a_bracket_expression_is_a_wildcard_pattern() {
    // procps treats bracket expressions as wildcard syntax:
    // `GLOB_CHARS` in procps-ng `src/sysctl.c` is `"*?["`, expanded
    // through libc `glob` — `vm.[m]ax_map_count` expands to
    // `vm.max_map_count` (measured on procps-ng 4.0.4 with
    // `sysctl --dry-run -p`). The matcher reads them as character
    // classes: literals, `a-z` ranges and `[!...]` negation. (Keys
    // compare canonicalized, the way the boot probe calls the
    // matcher.)
    assert!(boot::key_matches("vm.[m]ax_map_count", "vm/max_map_count"));
    assert!(boot::key_matches(
        "vm.max_map_coun[t-z]",
        "vm/max_map_count"
    ));
    assert!(!boot::key_matches(
        "vm.max_map_coun[a-s]",
        "vm/max_map_count"
    ));
    assert!(boot::key_matches(
        "vm.max_map_coun[!a-s]",
        "vm/max_map_count"
    ));
    assert!(!boot::key_matches(
        "vm.max_map_coun[!t-z]",
        "vm/max_map_count"
    ));
    // An unterminated class is unknown syntax: the conservative
    // answer assumes the assignment rather than miss an override.
    assert!(boot::key_matches("vm.max_map_count[", "vm/max_map_count"));
    // And a key without wildcard syntax is still a literal.
    assert!(!boot::key_matches("vm.max_map_count2", "vm/max_map_count"));
    assert!(boot::key_matches("vm.max_map_count", "vm/max_map_count"));
}

#[test]
fn a_bracket_assignment_names_no_key() {
    // A bracket expression is a pattern, not a name, so it weighs on
    // no named set: only the plain assignment names the key.
    assert_eq!(
        boot::named_keys(b"vm.[m]ax_map_count=65530\n"),
        Vec::<String>::new(),
        "a bracket assignment must not name the key"
    );
    assert_eq!(
        boot::named_keys(b"vm.max_map_count=65530\n"),
        vec!["vm/max_map_count".to_string()],
        "a plain assignment names the key"
    );
}

#[test]
fn a_bracket_pattern_never_reaches_a_named_key() {
    // Like every pattern, the bracket expansion never reaches a key
    // the sequence names outright (procps `settinglist_findpath`
    // skips the expansion for explicitly listed paths): the managed
    // file's own entry — the assignment `rewrite_file` is about to
    // write — keeps the boot value, whatever the later pattern holds.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "vm.max_map_count",
            "65530",
            &[("/etc/sysctl.conf", "vm.[m]ax_map_count=123\n")],
        ),
        None
    );
    // The `?` spelling of the same pattern is excluded the same way.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "vm.max_map_count",
            "65530",
            &[("/etc/sysctl.conf", "vm.max_map_coun?=123\n")],
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

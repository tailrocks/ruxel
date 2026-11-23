//! Boot-file separator canonicalization: dot-first mixed spellings.

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
    boot::boot_override(
        file,
        name,
        value,
        &host,
        &super::super::SysctlManaged::default(),
    )
}

#[test]
fn a_dot_first_mixed_spelling_in_a_boot_file_is_the_same_assignment() {
    // The boot scanner canonicalizes the way procps reads: a dot-first
    // mixed spelling in a later-loading file assigns the managed entry,
    // so its differing value hands the boot value away — and its equal
    // value hands boot the managed value, overriding nothing.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net/ipv4/conf/enp3s0.200/forwarding",
            "1",
            &[(
                "/etc/sysctl.conf",
                "net.ipv4.conf.enp3s0/200.forwarding = 0\n"
            )],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net/ipv4/conf/enp3s0.200/forwarding",
            "1",
            &[(
                "/etc/sysctl.conf",
                "net.ipv4.conf.enp3s0/200.forwarding = 1\n"
            )],
        ),
        None
    );
    // The managed name may be the mixed spelling too: the scanner
    // compares canonical keys on both sides.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.enp3s0/200.forwarding",
            "1",
            &[(
                "/etc/sysctl.conf",
                "net/ipv4/conf/enp3s0.200/forwarding = 0\n"
            )],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn a_literal_dot_keeps_its_identity_in_boot_files() {
    // A slash-first name's literal dot is not the dotted spelling of
    // another path, in boot files as in the managed entry: the later
    // file's dotted `enp3s0.200` assignment addresses `enp3s0/200` and
    // hands nothing away from the managed `enp3s0.200` component.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net/ipv4/conf/enp3s0.200/forwarding",
            "1",
            &[(
                "/etc/sysctl.conf",
                "net.ipv4.conf.enp3s0.200.forwarding=0\n"
            )],
        ),
        None
    );
}

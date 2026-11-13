//! The boot override: a persistence entry is refused when a
//! later-loading system file assigns the managed key to a value of its
//! own — the boot value would come from there and the parameter reverts
//! after reboot.

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
fn a_later_boot_assignment_overrides_the_persistence_file() {
    // The default managed file with `/etc/sysctl.conf` assigning the
    // key behind it: the system files load together by file name and
    // `/etc/sysctl.conf` after all of them, so reboot restores the
    // unmanaged value and the managed entry is not the boot value.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[
                ("/etc/sysctl.d/90-ruxel.conf", "net.ipv4.ip_forward=1\n"),
                ("/etc/sysctl.conf", "net.ipv4.ip_forward=0\n"),
            ],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn an_alias_spelling_in_the_later_file_is_the_same_assignment() {
    // Keys compare canonicalized: `net/ipv4/ip_forward` is the same
    // entry as `net.ipv4.ip_forward`, whatever spelling the later file
    // writes it in.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[("/etc/sysctl.conf", "net/ipv4/ip_forward = 0\n")],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn earlier_and_same_name_files_never_override() {
    // The system files sort together by their base names, so a smaller
    // base name loads first and the managed entry is the boot value. A
    // base name loads once, from its highest-precedence directory (the
    // shadow check refuses that case), so a same-named copy is not a
    // later load either — and another key is not this entry.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[
                ("/etc/sysctl.d/10-earlier.conf", "net.ipv4.ip_forward=0\n"),
                ("/usr/lib/sysctl.d/90-ruxel.conf", "net.ipv4.ip_forward=0\n"),
                ("/run/sysctl.d/90-ruxel.conf", "net.ipv4.ip_forward=0\n"),
                ("/etc/sysctl.d/99-other.conf", "net.test=0\n"),
            ],
        ),
        None
    );
}

#[test]
fn an_equal_later_assignment_is_the_same_boot_value() {
    // An assignment to the same value hands boot the managed value:
    // nothing reverts, whatever loads last.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[("/etc/sysctl.conf", "net.ipv4.ip_forward = 1\n")],
        ),
        None
    );
}

#[test]
fn the_etc_sysctl_conf_entry_is_the_last_load() {
    // The legacy file loads after every directory file: no later load
    // exists to hand the boot value away.
    assert_eq!(
        override_of(
            "/etc/sysctl.conf",
            "net.ipv4.ip_forward",
            "1",
            &[
                ("/etc/sysctl.d/90-ruxel.conf", "net.ipv4.ip_forward=0\n"),
                ("/etc/sysctl.conf", "net.ipv4.ip_forward=1\n"),
            ],
        ),
        None
    );
}

#[test]
fn a_dash_prefixed_assignment_is_the_same_entry() {
    // procps reads a leading '-' as the ignore-failure marker: the
    // assignment still happens, so the later file hands the boot value
    // away from the managed entry.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &[("/etc/sysctl.conf", "-net.ipv4.ip_forward=0\n")],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn a_wildcard_key_assigns_the_managed_key() {
    // A wildcard key expands to every key it covers, so a later file
    // that assigns `net.ipv4.conf.*.rp_filter` assigns the managed
    // concrete key `net.ipv4.conf.all.rp_filter` too.
    assert_eq!(
        override_of(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &[("/etc/sysctl.conf", "net.ipv4.conf.*.rp_filter=0\n")],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
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
fn a_boot_file_that_is_not_text_still_assigns() {
    // The assignment syntax is ASCII: a file that holds a byte
    // sequence that is not text is still read, and its assignments
    // weigh.
    let mut content = b"net.ipv4.ip_forward=0\n# comment ".to_vec();
    content.push(0xff);
    content.push(b'\n');
    let host = vec![("/etc/sysctl.conf".to_string(), content)];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &[],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn a_declared_managed_file_is_not_an_unmanaged_override() {
    // The later stage of a workflow that sets the same parameter on
    // purpose declares its file: the later value is the declared one,
    // not an unmanaged override, and the earlier stage must be able to
    // run again.
    let host = vec![(
        "/etc/sysctl.d/99-new.conf".to_string(),
        b"net.ipv4.ip_forward=0\n".to_vec(),
    )];
    let managed = vec![(
        "/etc/sysctl.d/99-new.conf".to_string(),
        "net.ipv4.ip_forward".to_string(),
        "0".to_string(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/10-old.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &managed,
        ),
        None
    );
}

#[test]
fn a_managed_file_that_drifted_is_an_unmanaged_override() {
    // The declared value is the one ruxel manages there: a file that
    // drifted to another value carries an unmanaged assignment again.
    let host = vec![(
        "/etc/sysctl.d/99-new.conf".to_string(),
        b"net.ipv4.ip_forward=2\n".to_vec(),
    )];
    let managed = vec![(
        "/etc/sysctl.d/99-new.conf".to_string(),
        "net.ipv4.ip_forward".to_string(),
        "0".to_string(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/10-old.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &managed,
        ),
        Some(("/etc/sysctl.d/99-new.conf".to_string(), "2".to_string()))
    );
}

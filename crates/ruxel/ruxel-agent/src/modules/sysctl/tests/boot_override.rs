//! The boot override: a persistence entry is refused when a
//! later-loading system file assigns the managed key to a value of its
//! own — the boot value would come from there and the parameter reverts
//! after reboot.

#[test]
fn a_later_boot_assignment_overrides_the_persistence_file() {
    // The default managed file with `/etc/sysctl.conf` assigning the
    // key behind it: the system files load together by file name and
    // `/etc/sysctl.conf` after all of them, so reboot restores the
    // unmanaged value and the managed entry is not the boot value.
    let host = vec![
        (
            "/etc/sysctl.d/90-ruxel.conf".to_string(),
            "net.ipv4.ip_forward=1\n".to_string(),
        ),
        (
            "/etc/sysctl.conf".to_string(),
            "net.ipv4.ip_forward=0\n".to_string(),
        ),
    ];
    assert_eq!(
        super::super::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn an_alias_spelling_in_the_later_file_is_the_same_assignment() {
    // Keys compare canonicalized: `net/ipv4/ip_forward` is the same
    // entry as `net.ipv4.ip_forward`, whatever spelling the later file
    // writes it in.
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        "net/ipv4/ip_forward = 0\n".to_string(),
    )];
    assert_eq!(
        super::super::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host
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
    let host = vec![
        (
            "/etc/sysctl.d/10-earlier.conf".to_string(),
            "net.ipv4.ip_forward=0\n".to_string(),
        ),
        (
            "/usr/lib/sysctl.d/90-ruxel.conf".to_string(),
            "net.ipv4.ip_forward=0\n".to_string(),
        ),
        (
            "/run/sysctl.d/90-ruxel.conf".to_string(),
            "net.ipv4.ip_forward=0\n".to_string(),
        ),
        (
            "/etc/sysctl.d/99-other.conf".to_string(),
            "net.test=0\n".to_string(),
        ),
    ];
    assert_eq!(
        super::super::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host
        ),
        None
    );
}

#[test]
fn an_equal_later_assignment_is_the_same_boot_value() {
    // An assignment to the same value hands boot the managed value:
    // nothing reverts, whatever loads last.
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        "net.ipv4.ip_forward = 1\n".to_string(),
    )];
    assert_eq!(
        super::super::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host
        ),
        None
    );
}

#[test]
fn the_etc_sysctl_conf_entry_is_the_last_load() {
    // The legacy file loads after every directory file: no later load
    // exists to hand the boot value away.
    let host = vec![
        (
            "/etc/sysctl.d/90-ruxel.conf".to_string(),
            "net.ipv4.ip_forward=0\n".to_string(),
        ),
        (
            "/etc/sysctl.conf".to_string(),
            "net.ipv4.ip_forward=1\n".to_string(),
        ),
    ];
    assert_eq!(
        super::super::boot_override("/etc/sysctl.conf", "net.ipv4.ip_forward", "1", &host),
        None
    );
}

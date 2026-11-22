//! Boot byte values: the comparison never decodes lossily.

use std::path::PathBuf;

use super::super::boot;

#[test]
fn non_utf8_boot_values_compare_as_bytes() {
    // procps passes the file's bytes to the kernel: a later `\xff`
    // byte and a requested U+FFFD are different boot values even
    // though `from_utf8_lossy` spells both `�` — and the managed
    // UTF-8 bytes in place are the requested value, not a lossy
    // collision to refuse. The diagnostic spells the differing bytes
    // lossily; the comparison never does.
    let mut later = b"kernel.core_pattern=".to_vec();
    later.push(0xff);
    later.push(b'\n');
    let host = vec![(PathBuf::from("/etc/sysctl.conf"), later)];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "kernel.core_pattern",
            "\u{fffd}",
            &host,
            &super::super::SysctlManaged::default(),
        ),
        Some(("/etc/sysctl.conf".to_string(), "\u{fffd}".to_string())),
        "a later byte sequence that is not the requested value overrides"
    );
    let host = vec![(
        PathBuf::from("/etc/sysctl.conf"),
        "kernel.core_pattern=\u{fffd}\n".as_bytes().to_vec(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "kernel.core_pattern",
            "\u{fffd}",
            &host,
            &super::super::SysctlManaged::default(),
        ),
        None,
        "the managed UTF-8 bytes are the requested value"
    );
}

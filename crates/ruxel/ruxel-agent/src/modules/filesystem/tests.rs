#[test]
fn parses_ext4_block_count_without_resize2fs_prose() {
    assert_eq!(
        super::parse_ext4_block_count("Filesystem volume name: x\nBlock count: 5240832\n"),
        Some(5_240_832)
    );
    assert_eq!(super::parse_ext4_block_count("Block size: 4096\n"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn validates_logical_sector_size() {
    assert_eq!(super::parse_sector_size(b"512\n").unwrap(), 512);
    assert_eq!(super::parse_sector_size(b"4096\n").unwrap(), 4096);
    for output in [&b"0"[..], &b"1023"[..], &b"invalid"[..]] {
        assert!(super::parse_sector_size(output).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn blank_and_filesystem_are_the_only_accepted_probe_states() {
    use rsblkid::probe::ScanResult;
    assert_eq!(
        super::classify_probe(ScanResult::NoProperties, false, None, None).unwrap(),
        None
    );
    assert_eq!(
        super::classify_probe(
            ScanResult::FoundProperties,
            false,
            Some("filesystem"),
            Some("ext4".into())
        )
        .unwrap(),
        Some("ext4".into())
    );
    assert!(
        super::classify_probe(
            ScanResult::FoundProperties,
            true,
            Some("filesystem"),
            Some("ext4".into())
        )
        .is_err()
    );
    assert!(
        super::classify_probe(
            ScanResult::FoundProperties,
            false,
            Some("crypto"),
            Some("crypto_LUKS".into())
        )
        .is_err()
    );
    assert!(
        super::classify_probe(ScanResult::FoundProperties, false, Some("filesystem"), None)
            .is_err()
    );
    for result in [
        ScanResult::Error,
        ScanResult::ConflictingValues,
        ScanResult::Exception(-5),
    ] {
        assert!(super::classify_probe(result, false, None, None).is_err());
    }
}

#[test]
fn refuses_xfs_growth_in_check_and_apply() {
    assert!(super::ensure_safe_resize("xfs").is_err());
    assert!(super::ensure_safe_resize("ext4").is_ok());
}

#[test]
fn predicts_resize_from_geometry() {
    assert!(!super::has_growth_space(4096 * 100, 4096, 100));
    assert!(super::has_growth_space(4096 * 101, 4096, 100));
    assert!(!super::has_growth_space(4096 * 100 + 100, 4096, 100));
}

#[test]
fn mkfs_program_is_allowlisted() {
    let error = super::mkfs_program("xfs;touch /tmp/pwn").unwrap_err();
    assert!(error.contains("outside the closed surface"));
    assert_eq!(
        super::parse_xfs_block_count(
            "data     =                       bsize=4096   blocks=5242880, imaxpct=25\n"
        ),
        Some(5_242_880)
    );
    assert_eq!(super::parse_xfs_block_count("meta-data= blocks=9\n"), None);
}

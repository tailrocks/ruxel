#[test]
fn synthetic_perl_ioctl_growth_and_guards() {
    let script = format!(
        "{}\n{}",
        include_str!("../xfs.pl"),
        include_str!("xfs_synthetic.pl")
    );
    let output = std::process::Command::new("perl")
        .args(["-T", "-e", &script])
        .env_remove("PERL5OPT")
        .env_remove("PERL5LIB")
        .env_remove("PERLLIB")
        .env_remove("PERL5DB")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ok\n");
}
#[test]
fn perl_and_required_core_modules_are_available() {
    let result = std::process::Command::new("perl")
        .args([
            "-T",
            "-MConfig",
            "-MFcntl",
            "-MMath::BigInt",
            "-MPOSIX",
            "-e",
            "1",
        ])
        .output()
        .expect("XFS tests require Perl and its core modules");
    assert!(
        result.status.success(),
        "XFS tests require Perl core modules: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

#[test]
fn handoff_preserves_directory_identity_and_replaces_stdin() {
    let directory = std::fs::File::open(".").unwrap();
    let metadata = directory.metadata().unwrap();
    let code = format!(
        "use Fcntl ':mode'; open my $f, '<&=3' or die $!; my @s = stat($f); \
         die 'wrong fd3' unless S_ISDIR($s[2]) && $s[0] == {} && $s[1] == {}; \
         my @in = stat(STDIN); die 'directory stdin' if S_ISDIR($in[2]); \
         my $byte; die 'stdin not empty' if read(STDIN, $byte, 1);",
        metadata.dev(),
        metadata.ino()
    );
    let output = Command::new("sh")
        .args([
            "-c",
            super::HANDOFF,
            "ruxel-xfs-helper",
            "perl",
            "-T",
            "-e",
            &code,
        ])
        .stdin(Stdio::from(directory))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn production_helper_reports_real_geometry_ioctl_error() {
    // procfs rejects XFS geometry requests; no block device is opened.
    let directory = std::fs::File::open("/proc/self").unwrap();
    let device_id = directory.metadata().unwrap().dev().to_string();
    let output = Command::new("sh")
        .args([
            "-c",
            super::HANDOFF,
            "ruxel-xfs-helper",
            "perl",
            "-T",
            "-e",
            super::HELPER,
            "preflight",
            "0",
            &device_id,
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::from(directory))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("XFS ioctl failed: Inappropriate ioctl for device"),
        "{error}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn real_perl_starts_with_directory_descriptor() {
    let directory = std::fs::File::open(".").unwrap();
    let error =
        super::invoke_with_program(&directory, &directory, "preflight", 0, "perl").unwrap_err();
    assert!(
        error.contains("mount descriptor identity mismatch"),
        "{error}"
    );
}

#[test]
fn missing_interpreter_fails_before_any_ioctl() {
    let directory = std::fs::File::open(".").unwrap();
    let error = super::invoke_with_program(
        &directory,
        &directory,
        "preflight",
        0,
        "/__ruxel_missing_perl__",
    )
    .unwrap_err();
    assert!(error.contains("XFS helper failed"));
}

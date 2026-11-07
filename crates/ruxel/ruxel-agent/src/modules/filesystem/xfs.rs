use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

const HELPER: &str = concat!(include_str!("xfs.pl"), "\nmain_xfs(@ARGV);\n");
const HANDOFF: &str = "exec 3<&0; exec </dev/null; exec \"$@\"";

pub(super) fn prepare(device: &std::fs::File) -> Result<std::fs::File, String> {
    let mount = super::xfs_mount(device)?;
    invoke(device, &mount, "preflight", 0)?;
    Ok(mount)
}

pub(super) fn resize(
    device: &std::fs::File,
    mount: &std::fs::File,
    check: bool,
) -> Result<bool, String> {
    let output = super::run_on_file(device, "blockdev", &["--getsize64"])?;
    let bytes: u64 = std::str::from_utf8(&output.stdout)
        .map_err(|_| "filesystem: invalid device size")?
        .trim()
        .parse()
        .map_err(|_| "filesystem: invalid device size")?;
    invoke(device, mount, if check { "check" } else { "grow" }, bytes)
}

fn invoke(
    device: &std::fs::File,
    mount: &std::fs::File,
    mode: &str,
    bytes: u64,
) -> Result<bool, String> {
    invoke_with_program(device, mount, mode, bytes, "perl")
}

fn invoke_with_program(
    device: &std::fs::File,
    mount: &std::fs::File,
    mode: &str,
    bytes: u64,
    program: &str,
) -> Result<bool, String> {
    let rdev = device.metadata().map_err(|e| e.to_string())?.rdev();
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            HANDOFF,
            "ruxel-xfs-helper",
            program,
            "-T",
            "-e",
            HELPER,
            mode,
            &bytes.to_string(),
            &rdev.to_string(),
        ])
        .env_remove("PERL5OPT")
        .env_remove("PERL5LIB")
        .env_remove("PERLLIB")
        .env_remove("PERL5DB")
        .stdin(Stdio::from(mount.try_clone().map_err(|e| e.to_string())?));
    let output = command
        .output()
        .map_err(|e| format!("filesystem: XFS helper requires perl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "filesystem: XFS helper failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("filesystem: invalid XFS helper result: {e}"))?;
    result["changed"]
        .as_bool()
        .ok_or_else(|| "filesystem: missing XFS helper result".into())
}

#[cfg(test)]
mod tests {
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

    #[test]
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
}

#[cfg(test)]
#[path = "tests/xfs_perl.rs"]
mod synthetic_tests;

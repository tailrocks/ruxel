use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

const HELPER: &str = include_str!("xfs.py");

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
    invoke_with_program(device, mount, mode, bytes, "python3")
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
            "exec 3<&0; exec </dev/null; exec \"$@\"",
            "ruxel-xfs-helper",
            program,
            "-I",
            "-S",
            "-c",
            HELPER,
            mode,
            &bytes.to_string(),
            &rdev.to_string(),
        ])
        .stdin(Stdio::from(mount.try_clone().map_err(|e| e.to_string())?));
    let output = command
        .output()
        .map_err(|e| format!("filesystem: XFS helper requires python3: {e}"))?;
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
    #[test]
    fn real_python_starts_with_directory_descriptor() {
        let directory = std::fs::File::open(".").unwrap();
        let error = super::invoke_with_program(&directory, &directory, "preflight", 0, "python3")
            .unwrap_err();
        assert!(
            error.contains("mount descriptor identity mismatch"),
            "{error}"
        );
        assert!(!error.contains("init_sys_streams"));
    }

    #[test]
    fn missing_interpreter_fails_before_any_ioctl() {
        let directory = std::fs::File::open(".").unwrap();
        let error = super::invoke_with_program(
            &directory,
            &directory,
            "preflight",
            0,
            "/__ruxel_missing_python3__",
        )
        .unwrap_err();
        assert!(error.contains("XFS helper failed"));
    }
}

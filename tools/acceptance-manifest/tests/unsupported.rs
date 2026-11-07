use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[test]
fn unsupported_case_fails_without_launching_cli() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir_all(temp.path().join("testdata/workloads/pr66"))?;
    fs::write(
        temp.path()
            .join("testdata/workloads/pr66/expectations.json"),
        include_bytes!("../../../testdata/workloads/pr66/expectations.json"),
    )?;
    let marker = temp.path().join("launched");
    let stub = temp.path().join("cli");
    fs::write(&stub, format!("#!/bin/sh\ntouch '{}'\n", marker.display()))?;
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700))?;
    let result = Command::new(env!("CARGO_BIN_EXE_ruxel-acceptance-manifest"))
        .arg("--root")
        .arg(temp.path())
        .arg("--ruxel")
        .arg(&stub)
        .arg("--agent")
        .arg(&stub)
        .args(["--scenario", "storage-ext4"])
        .output()?;
    assert!(!result.status.success());
    assert!(!marker.exists());
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("no reviewed safe local adapter"),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

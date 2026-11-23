#![forbid(unsafe_code)]
use std::process::Command;
#[test]
fn ambient_proxy_environment_cannot_skip_acceptance() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel-acceptance-runtime"))
        .env("RUXEL_RUNTIME_PROXY_AGENT", "/usr/bin/true")
        .env("RUXEL_RUNTIME_PROXY_LABELS", temp.path().join("labels"))
        .args(["--ruxel", "/usr/bin/true", "--agent", "/usr/bin/true"])
        .output()?;
    assert!(
        !output.status.success(),
        "ambient proxy variables skipped all scenarios"
    );
    Ok(())
}

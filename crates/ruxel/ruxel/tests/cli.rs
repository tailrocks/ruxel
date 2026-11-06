use std::process::Command;

#[test]
fn dump_and_missing_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout)?,
        serde_json::json!({"name":"ruxel", "count":1})
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", "/nonexistent.pkl"])
        .output()?;
    assert_eq!(output.status.code(), Some(4));
    Ok(())
}

#[test]
fn raw_dump_preserves_declaration_order() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let plain = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .output()?;
    let raw = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root, "--raw"])
        .output()?;
    assert!(raw.status.success());
    assert_eq!(raw.stdout, plain.stdout);
    assert_eq!(
        String::from_utf8(raw.stdout)?,
        "{\n  \"name\": \"ruxel\",\n  \"count\": 1\n}\n"
    );
    Ok(())
}

#[test]
fn version_and_usage_codes() -> Result<(), Box<dyn std::error::Error>> {
    let version = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .arg("--version")
        .output()?;
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout)?,
        format!("ruxel {}\n", env!("RUXEL_VERSION"))
    );
    let usage = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump"])
        .output()?;
    assert_eq!(usage.status.code(), Some(2));
    Ok(())
}

#[cfg(unix)]
#[test]
fn unwritable_stdout_is_internal_error() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let (writer, reader) = std::os::unix::net::UnixStream::pair()?;
    drop(reader);
    let writer = std::os::fd::OwnedFd::from(writer);
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .stdout(writer)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

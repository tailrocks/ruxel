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

#[test]
fn execution_rejects_untyped_output_as_config_error() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["plan", root])
        .output()?;
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stdout, Vec::<u8>::new());
    assert_eq!(output.stderr, b"configuration validation failed\n");
    Ok(())
}

#[test]
fn detailed_plan_counts_only_selected_operations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("ruxel-cli-plan-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../schema/Ruxel.pkl"),
        directory.join("Ruxel.pkl"),
    )?;
    let root = directory.join("plan.pkl");
    std::fs::write(
        &root,
        r#"import "Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "plan"; targets { "fixture" }; tasks { new {
  name = "deferred"; tags { "selected" }; operation = new Ruxel.Debug { msg = "fixture" }
 } } } }
} }
"#,
    )?;
    for (tag, expected, planned) in [("selected", 3, 1), ("other", 0, 0)] {
        let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
            .arg("plan")
            .arg(&root)
            .args([
                "--detailed-exitcode",
                "--tags",
                tag,
                "--agent",
                "/missing-agent",
            ])
            .output()?;
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert!(
            events
                .iter()
                .any(|event| event["recap"]["planned"] == planned)
        );
    }
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

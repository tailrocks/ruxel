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

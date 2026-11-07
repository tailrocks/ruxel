use super::*;
use serde_json::json;
#[test]
fn malformed_events_do_not_pass_assertions() {
    let good =
        json!({"host":"fixture","task":"copy","result":{"changed":true},"recap":{"failed":0}});
    assert!(validate_recap(std::slice::from_ref(&good)).is_ok());
    assert!(task_result(std::slice::from_ref(&good), "copy").is_ok());
    for events in [
        vec![],
        vec![good.clone(), good],
        vec![json!({"host":"fixture","recap":{"failed":1}})],
    ] {
        assert!(validate_recap(&events).is_err());
    }
    assert!(task_result(&[], "copy").is_err());
}
#[test]
fn snapshot_detects_binary_and_mode_drift() -> Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir(temp.path().join("schema"))?;
    fs::write(temp.path().join("schema/Ruxel.pkl"), "")?;
    let harness = Harness::new(temp.path(), Path::new("unused"), Path::new("unused"))?;
    let file = harness.payload.join("binary");
    fs::write(&file, [0, 255])?;
    let before = harness.snapshot()?;
    fs::write(&file, [0, 254])?;
    assert_ne!(before, harness.snapshot()?);
    fs::write(&file, [0, 255])?;
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640))?;
    let before = harness.snapshot()?;
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
    assert_ne!(before, harness.snapshot()?);
    Ok(())
}

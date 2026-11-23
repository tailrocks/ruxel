use super::{events, file, metadata, preview, validate_inputs};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::symlink};

#[test]
fn fixture_and_contract_must_match_exactly() -> anyhow::Result<()> {
    let fixture = include_bytes!("../../../testdata/workloads/pr66/restart-actions.pkl");
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../testdata/workloads/pr66/expectations.json"
    ))?;
    let mut case = manifest["scenarios"]["restart-actions"].clone();
    validate_inputs(fixture, &case)?;
    assert!(validate_inputs(b"changed", &case).is_err());
    case["phases"]["fresh"]["observations_match"] = json!(false);
    assert!(validate_inputs(fixture, &case).is_err());
    Ok(())
}
#[test]
fn rejects_incorrect_metadata_and_links() -> anyhow::Result<()> {
    let bytes = b"#!/bin/sh\nexit 0\n";
    metadata(true, 0, 0, 0o755, bytes)?;
    for (regular, uid, gid, mode, contents) in [
        (false, 0, 0, 0o755, bytes.as_slice()),
        (true, 1, 0, 0o755, bytes.as_slice()),
        (true, 0, 1, 0o755, bytes.as_slice()),
        (true, 0, 0, 0o700, bytes.as_slice()),
        (true, 0, 0, 0o755, b"drift".as_slice()),
    ] {
        assert!(metadata(regular, uid, gid, mode, contents).is_err());
    }
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("file"), bytes)?;
    symlink("file", dir.path().join("link"))?;
    assert!(file(&dir.path().join("link")).is_err());
    Ok(())
}
#[test]
fn requires_all_ordered_loop_results_and_exact_recap() -> anyhow::Result<()> {
    let items: Vec<_> = (1..=36)
        .map(|i| json!({"item":format!("{i:02}"),"rc":0}))
        .collect();
    let mut rows = vec![
        json!({"host":"fixture","recap":{"failed":0}}),
        json!({"host":"fixture","task":"Restart synthetic workload units","result":{"results":items}}),
    ];
    events(&rows)?;
    for field in ["censored", "redacted", "skipped", "failed"] {
        rows[1]["result"][field] = json!(true);
        assert!(events(&rows).is_err());
        rows[1]["result"][field] = json!(false);
        rows[1]["result"]["results"][0][field] = json!(true);
        assert!(events(&rows).is_err());
        rows[1]["result"]["results"][0][field] = json!(false);
    }
    rows[1]["result"]["results"][0]["rc"] = json!(1);
    assert!(events(&rows).is_err());
    rows[1]["result"]["results"][0]["rc"] = json!(0);
    rows[1]["result"]["results"][0]["item"] = json!("02");
    assert!(events(&rows).is_err());
    rows[1]["result"]["results"][0]["item"] = json!("01");
    rows[1]["result"]["results"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("missing array"))?
        .pop();
    assert!(events(&rows).is_err());
    rows[1]["result"]["results"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("missing array"))?
        .push(json!({"item":"36","rc":0}));
    events(&rows)?;
    rows.push(rows[0].clone());
    assert!(events(&rows).is_err());
    Ok(())
}
#[test]
fn preview_requires_snapshot_equality() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("restart"), b"drift")?;
    let before = crate::state::fixture_snapshot(temp.path())?;
    preview(&before, &crate::state::fixture_snapshot(temp.path())?)?;
    fs::write(temp.path().join("restart"), b"changed")?;
    assert!(preview(&before, &crate::state::fixture_snapshot(temp.path())?).is_err());
    Ok(())
}

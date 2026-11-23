use super::{events, no_leak, scan, task};
use std::fs;
#[test]
fn rejects_leaks_duplicate_results_and_invalid_state() -> anyhow::Result<()> {
    let marker = "synthetic-test-private";
    assert!(no_leak("public output", &[marker]).is_ok());
    assert!(no_leak(marker, &[marker]).is_err());
    let rows = events("{\"task\":\"x\",\"result\":{}}\n{\"task\":\"x\",\"result\":{}}\n")?;
    assert!(task(&rows, "x").is_err());
    let rows = events("{\"task\":\"x\",\"result\":{}}\n{\"task\":\"x\"}\n")?;
    assert!(task(&rows, "x").is_err());

    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("state"), marker)?;
    assert!(scan(temp.path(), &[marker], false).is_err());
    fs::write(temp.path().join("state"), "safe")?;
    fs::write(temp.path().join("ledger.json"), "safe")?;
    assert!(scan(temp.path(), &[marker], true).is_err());
    assert!(scan(temp.path(), &[marker], false).is_ok());
    Ok(())
}

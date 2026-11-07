use super::fixture_snapshot;
use std::fs;
#[test]
fn snapshot_detects_fixture_changes_but_ignores_agent_state() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir(temp.path().join("schema"))?;
    fs::create_dir(temp.path().join("state"))?;
    fs::write(temp.path().join("schema/Ruxel.pkl"), "schema")?;
    fs::write(temp.path().join("state/ledger"), "one")?;
    let before = fixture_snapshot(temp.path())?;
    fs::write(temp.path().join("state/ledger"), "two")?;
    assert_eq!(fixture_snapshot(temp.path())?, before);
    fs::write(temp.path().join("schema/Ruxel.pkl"), "changed")?;
    assert_ne!(fixture_snapshot(temp.path())?, before);
    Ok(())
}

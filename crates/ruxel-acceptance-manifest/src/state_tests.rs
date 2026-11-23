use super::{Entry, fixture_snapshot, snapshot};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
#[test]
fn snapshot_detects_fixture_changes_but_ignores_agent_state() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir(temp.path().join("fixtures"))?;
    fs::create_dir(temp.path().join("state"))?;
    fs::write(temp.path().join("fixtures/workload.pkl"), "fixture")?;
    fs::write(temp.path().join("state/ledger"), "one")?;
    let before = fixture_snapshot(temp.path())?;
    fs::write(temp.path().join("state/ledger"), "two")?;
    assert_eq!(fixture_snapshot(temp.path())?, before);
    fs::write(temp.path().join("fixtures/workload.pkl"), "changed")?;
    assert_ne!(fixture_snapshot(temp.path())?, before);
    Ok(())
}

#[test]
fn snapshot_captures_root_and_entry_ownership_and_metadata() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("file"), b"content")?;
    let before = snapshot(temp.path())?;
    let root = before
        .get(Path::new(""))
        .ok_or_else(|| anyhow::anyhow!("missing root"))?;
    let Entry::Directory(root_metadata) = root else {
        anyhow::bail!("invalid root")
    };
    let mut changed = root_metadata.clone();
    changed.uid = changed.uid.wrapping_add(1);
    assert_ne!(&changed, root_metadata);
    changed = root_metadata.clone();
    changed.gid = changed.gid.wrapping_add(1);
    assert_ne!(&changed, root_metadata);
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o750))?;
    assert_ne!(snapshot(temp.path())?, before);

    let before_file = snapshot(temp.path())?;
    let entry = before_file
        .get(Path::new("file"))
        .ok_or_else(|| anyhow::anyhow!("missing file"))?;
    let Entry::File(metadata, _) = entry else {
        anyhow::bail!("invalid file")
    };
    let mut changed = metadata.clone();
    changed.uid = changed.uid.wrapping_add(1);
    assert_ne!(&changed, metadata);
    changed = metadata.clone();
    changed.gid = changed.gid.wrapping_add(1);
    assert_ne!(&changed, metadata);
    fs::set_permissions(temp.path().join("file"), fs::Permissions::from_mode(0o600))?;
    assert_ne!(snapshot(temp.path())?, before_file);
    Ok(())
}

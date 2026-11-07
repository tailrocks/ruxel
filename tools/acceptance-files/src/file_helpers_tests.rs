use super::{Entry, snapshot};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

#[test]
fn snapshot_captures_content_modes_and_links_without_following() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("file"), b"one")?;
    fs::set_permissions(root.path().join("file"), fs::Permissions::from_mode(0o600))?;
    symlink("file", root.path().join("link"))?;
    let before = snapshot(root.path())?;
    assert_eq!(
        before.get(&PathBuf::from("file")),
        Some(&Entry::File(0o600, b"one".to_vec()))
    );
    assert_eq!(
        before.get(&PathBuf::from("link")),
        Some(&Entry::Link(PathBuf::from("file")))
    );
    fs::set_permissions(root.path().join("file"), fs::Permissions::from_mode(0o640))?;
    assert_ne!(before, snapshot(root.path())?);
    Ok(())
}

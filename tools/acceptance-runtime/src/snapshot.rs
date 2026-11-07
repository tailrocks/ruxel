use anyhow::Result;
use std::{collections::BTreeMap, ffi::OsString, fs, os::unix::fs::MetadataExt, path::Path};
pub fn files(path: &Path) -> Result<BTreeMap<OsString, Vec<u8>>> {
    let mut snapshot = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.path().is_file() {
            snapshot.insert(entry.file_name(), fs::read(entry.path())?);
        }
    }
    Ok(snapshot)
}
pub fn files_with_modes(path: &Path) -> Result<BTreeMap<OsString, (u32, Vec<u8>)>> {
    let mut snapshot = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::metadata(entry.path())?;
        if metadata.is_file() {
            snapshot.insert(
                entry.file_name(),
                (metadata.mode(), fs::read(entry.path())?),
            );
        }
    }
    Ok(snapshot)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn detects_extra_files_and_mode_changes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let first = temp.path().join("first");
        fs::write(&first, "one")?;
        let bytes = files(temp.path())?;
        let modes = files_with_modes(temp.path())?;
        fs::set_permissions(&first, fs::Permissions::from_mode(0o600))?;
        if modes == files_with_modes(temp.path())? {
            fs::set_permissions(&first, fs::Permissions::from_mode(0o700))?;
        }
        assert_ne!(modes, files_with_modes(temp.path())?);
        assert_eq!(bytes, files(temp.path())?);
        fs::write(temp.path().join("unexpected"), "two")?;
        assert_ne!(bytes, files(temp.path())?);
        assert_ne!(modes, files_with_modes(temp.path())?);
        Ok(())
    }
}

use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Debug, PartialEq, Eq)]
pub enum Entry {
    Directory(u32),
    File(u32, Vec<u8>),
    Link(PathBuf),
}
fn walk(
    root: &Path,
    dir: &Path,
    entries: &mut BTreeMap<PathBuf, Entry>,
    exclude_state: bool,
) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        if exclude_state && path == root.join("state") {
            continue;
        }
        let info = fs::symlink_metadata(&path)?;
        let value = if info.file_type().is_symlink() {
            Entry::Link(fs::read_link(&path)?)
        } else if info.is_dir() {
            walk(root, &path, entries, exclude_state)?;
            Entry::Directory(info.mode() & 0o7777)
        } else if info.is_file() {
            Entry::File(info.mode() & 0o7777, fs::read(&path)?)
        } else {
            bail!("unexpected payload entry")
        };
        entries.insert(path.strip_prefix(root)?.to_path_buf(), value);
    }
    Ok(())
}
pub fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let mut result = BTreeMap::new();
    walk(root, root, &mut result, false)?;
    Ok(result)
}
pub fn fixture_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let mut result = BTreeMap::new();
    walk(root, root, &mut result, true)?;
    Ok(result)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

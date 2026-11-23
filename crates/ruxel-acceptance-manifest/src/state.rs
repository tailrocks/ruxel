use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    mode: u32,
    uid: u32,
    gid: u32,
    inode: u64,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}
impl Metadata {
    fn from(info: &fs::Metadata) -> Self {
        Self {
            mode: info.mode() & 0o7777,
            uid: info.uid(),
            gid: info.gid(),
            inode: info.ino(),
            size: info.size(),
            mtime: info.mtime(),
            mtime_nsec: info.mtime_nsec(),
            ctime: info.ctime(),
            ctime_nsec: info.ctime_nsec(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Directory(Metadata),
    File(Metadata, Vec<u8>),
    Link(Metadata, PathBuf),
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
            Entry::Link(Metadata::from(&info), fs::read_link(&path)?)
        } else if info.is_dir() {
            walk(root, &path, entries, exclude_state)?;
            Entry::Directory(Metadata::from(&info))
        } else if info.is_file() {
            Entry::File(Metadata::from(&info), fs::read(&path)?)
        } else {
            bail!("unexpected payload entry")
        };
        entries.insert(path.strip_prefix(root)?.to_path_buf(), value);
    }
    Ok(())
}
pub fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    snapshot_inner(root, false)
}
pub fn fixture_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    snapshot_inner(root, true)
}
fn snapshot_inner(root: &Path, exclude_state: bool) -> Result<BTreeMap<PathBuf, Entry>> {
    let info = fs::symlink_metadata(root)?;
    if !info.file_type().is_dir() {
        bail!("snapshot root is not a directory");
    }
    let mut result = BTreeMap::new();
    result.insert(PathBuf::new(), Entry::Directory(Metadata::from(&info)));
    walk(root, root, &mut result, exclude_state)?;
    Ok(result)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

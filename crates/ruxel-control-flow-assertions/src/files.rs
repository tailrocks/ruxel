use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
pub fn target(root: &Path, value: &str) -> Result<PathBuf> {
    let relative = value
        .strip_prefix("__RUN_ROOT__/")
        .context("path must be under __RUN_ROOT__")?;
    ensure!(
        !relative.starts_with('/')
            && relative.split('/').any(|p| !p.is_empty() && p != ".")
            && !relative.split('/').any(|p| p == ".."),
        "invalid fixture path"
    );
    let path = root.join(relative);
    let canonical_root = root.canonicalize()?;
    let canonical = path.canonicalize().context("missing observed file")?;
    ensure!(
        canonical.starts_with(canonical_root),
        "fixture path escapes root"
    );
    let meta = fs::symlink_metadata(&path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "missing or unsafe observed file"
    );
    Ok(path)
}
pub fn owner(path: &Path, name: &str, resolve: &impl Fn(&str) -> Result<u32>) -> Result<()> {
    let uid = resolve(name)?;
    ensure!(fs::metadata(path)?.uid() == uid, "file owner mismatch");
    Ok(())
}

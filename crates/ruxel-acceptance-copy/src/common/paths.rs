use anyhow::{Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn pkl(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
pub fn path(path: &Path) -> Result<String> {
    pkl(path.to_str().context("path is not UTF-8")?)
}
pub fn shell(path: &Path) -> Result<String> {
    Ok(format!(
        "'{}'",
        path.to_str()
            .context("path is not UTF-8")?
            .replace('\'', "'\\''")
    ))
}
pub fn mode(path: &Path) -> Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o7777)
}

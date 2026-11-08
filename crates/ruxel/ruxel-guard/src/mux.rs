use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

pub(crate) fn prepare(path: &Path) -> Result<PathBuf, String> {
    crate::resolve::validate_path(path)?;
    let name = path.file_name().ok_or("invalid mux directory")?;
    let parent = path.parent().ok_or("missing mux parent")?;
    crate::trusted_path::ancestors(parent)?;
    let path = parent.join(name);
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.to_string()),
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err("mux directory must be owned by the effective user and have mode 0700".into());
    }
    crate::trusted_path::writable_only_by_owner(&path, &metadata)?;
    Ok(path)
}

#[cfg(test)]
pub(crate) fn test_directory() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("test fixture requires HOME")?;
    let home = PathBuf::from(home)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    crate::trusted_path::ancestors(&home)?;
    if fs::metadata(&home).map_err(|e| e.to_string())?.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("test fixture requires an owned HOME".into());
    }
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    let suffix = format!("{:016x}", u64::from_ne_bytes(bytes));
    let path = home.join(format!(".ruxel-test-{suffix}"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn rejects_untrusted_ancestor_and_final_symlink() {
        let root = test_directory().unwrap();
        let mux = prepare(&root.join("mux")).unwrap();
        fs::remove_dir(mux).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(prepare(&root.join("mux")).is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let private = root.join("private");
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&private, root.join("link")).unwrap();
        assert!(prepare(&root.join("link")).is_err());
        fs::remove_file(root.join("link")).unwrap();
        fs::remove_dir(private).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

pub(crate) fn prepare(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("invalid mux directory")?;
    let parent = path
        .parent()
        .ok_or("missing mux parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    validate_ancestors(&parent)?;
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
    Ok(path)
}

fn validate_ancestors(path: &Path) -> Result<(), String> {
    let uid = rustix::process::geteuid().as_raw();
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || (metadata.uid() != 0 && metadata.uid() != uid)
            || metadata.mode() & 0o022 != 0
        {
            return Err("mux ancestor must be trusted and not writable by group or others".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn rejects_untrusted_ancestor_and_final_symlink() {
        let root = std::env::temp_dir().join(format!("muxcheck{}", std::process::id()));
        fs::create_dir(&root).unwrap();
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

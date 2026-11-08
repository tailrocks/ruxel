use std::{
    fs::{self, Metadata, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

/// Check from the root down: once a directory has passed, another uid cannot
/// replace the next component while it is inspected or later used by OpenSSH.
/// The effective uid and root are trusted, as with the SSH identity itself.
pub(crate) fn ancestors(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("SSH trust paths must not contain parent traversal".into());
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("SSH trust ancestor must be a directory, without symlinks".into());
        }
        writable_only_by_owner(&current, &metadata)?;
    }
    Ok(())
}

pub(crate) fn writable_only_by_owner(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if (metadata.uid() != 0 && metadata.uid() != rustix::process::geteuid().as_raw())
        || metadata.mode() & 0o022 != 0
    {
        return Err(
            "SSH trust path must have trusted ownership and no group or other write access".into(),
        );
    }
    acl(path)
}

#[cfg(target_os = "macos")]
fn acl(path: &Path) -> Result<(), String> {
    let filesystem = rustix::fs::statfs(path).map_err(|e| e.to_string())?;
    let name = filesystem.f_fstypename.map(|byte| byte.to_ne_bytes()[0]);
    // Darwin sys/mount.h MNT_LOCAL. Reject network/unknown ACL providers.
    if filesystem.f_flags & 0x1000 == 0
        || !(name.starts_with(b"apfs\0") || name.starts_with(b"hfs\0"))
    {
        return Err("SSH trust path requires local APFS or HFS on macOS".into());
    }
    // Unlike POSIX ACL masks, macOS allow entries can grant rights beyond mode
    // bits, including deletion and ACL changes. Refuse every allow entry rather
    // than approximating their ordered permissions or inheritance semantics.
    let entries = exacl::getfacl(path, None).map_err(|e| e.to_string())?;
    if entries.iter().any(|entry| entry.allow) {
        return Err("SSH trust path must not have macOS ACL allow entries".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn acl(path: &Path) -> Result<(), String> {
    // On Linux POSIX ACLs, group mode bits represent ACL_MASK; clearing group
    // write therefore also excludes writes from named-user and group entries.
    // Restrict that reasoning to filesystems implementing these semantics.
    // NFS, CIFS, FUSE and unknown filesystems fail closed, on every component.
    let kind = rustix::fs::statfs(path).map_err(|e| e.to_string())?.f_type;
    if supported_linux_filesystem(kind) {
        Ok(())
    } else {
        Err(format!(
            "SSH trust path has unsupported Linux filesystem type {kind:#x}"
        ))
    }
}

#[cfg(target_os = "linux")]
fn supported_linux_filesystem(kind: rustix::fs::FsWord) -> bool {
    // Linux include/uapi/linux/magic.h: ext2/3/4, XFS, Btrfs, tmpfs and ramfs.
    // Overlayfs is excluded because lower layers may have other ACL
    // semantics and can be modified through their original backing paths.
    matches!(
        kind,
        0xef53 | 0x5846_5342 | 0x9123_683e | 0x0102_1994 | 0x8584_58f6
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn acl(_path: &Path) -> Result<(), String> {
    Err("SSH trust path validation supports only Linux and macOS".into())
}

pub(crate) fn known_hosts(path: &Path) -> Result<(), String> {
    crate::resolve::validate_path(path)?;
    // ParentDir must be rejected before asking the OS to resolve the parent.
    if path.components().any(|part| part == Component::ParentDir) {
        return Err("SSH trust paths must not contain parent traversal".into());
    }
    let parent = path.parent().ok_or("known_hosts must have a parent")?;
    ancestors(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_known_hosts(path, &metadata),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // fopen(a+) in OpenSSH creates with 0666, which may inherit a Linux
            // default ACL despite umask. Precreate atomically with mode 0600:
            // the kernel caps the inherited ACL mask using this explicit mode.
            // On macOS the checked parent can only pass deny entries to it.
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|e| e.to_string())?;
            validate_known_hosts(path, &file.metadata().map_err(|e| e.to_string())?)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn validate_known_hosts(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_file() {
        return Err("known_hosts must be a regular file, without symlinks".into());
    }
    writable_only_by_owner(path, metadata)
}

#[cfg(test)]
mod tests;

use std::path::Path;

#[cfg(target_os = "macos")]
pub(super) fn filesystem_and_acl(path: &Path) -> Result<(), String> {
    let filesystem = rustix::fs::statfs(path).map_err(|error| error.to_string())?;
    let name = filesystem.f_fstypename.map(|byte| byte.to_ne_bytes()[0]);
    // Darwin MNT_LOCAL: only local filesystems with understood ACL semantics.
    if !supported_macos_filesystem(filesystem.f_flags & 0x1000 != 0, &name) {
        return Err("get_url: trusted clients require local APFS or HFS on macOS".into());
    }
    // Darwin ACL grants can exceed the Unix mode bits, including rights to
    // delete or replace entries. Reject grants rather than interpreting them.
    let entries = exacl::getfacl(path, None).map_err(|error| error.to_string())?;
    if entries.iter().any(|entry| entry.allow) {
        return Err("get_url: trusted client paths must not have macOS ACL allow entries".into());
    }
    Ok(())
}

#[cfg(any(test, target_os = "macos"))]
fn supported_macos_filesystem(local: bool, name: &[u8]) -> bool {
    local && (name.starts_with(b"apfs\0") || name.starts_with(b"hfs\0"))
}

#[cfg(target_os = "linux")]
pub(super) fn filesystem_and_acl(path: &Path) -> Result<(), String> {
    // POSIX ACL masks are reflected in group mode bits on these filesystems.
    // Network, FUSE, overlay and unknown backing cannot support that proof.
    let kind = rustix::fs::statfs(path)
        .map_err(|error| error.to_string())?
        .f_type;
    if supported_linux_filesystem(kind) {
        Ok(())
    } else {
        Err(format!(
            "get_url: unsupported client filesystem type {kind:#x}"
        ))
    }
}

#[cfg(target_os = "linux")]
fn supported_linux_filesystem(kind: rustix::fs::FsWord) -> bool {
    // Linux magic.h: ext2/3/4, XFS, Btrfs, tmpfs and ramfs.
    matches!(
        kind,
        0xef53 | 0x5846_5342 | 0x9123_683e | 0x0102_1994 | 0x8584_58f6
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn filesystem_and_acl(_path: &Path) -> Result<(), String> {
    Err("get_url: trusted client validation supports only Linux and macOS".into())
}

#[cfg(test)]
mod tests;

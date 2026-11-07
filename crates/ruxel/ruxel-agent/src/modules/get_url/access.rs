use rustix::fs::OFlags;
#[cfg(target_vendor = "apple")]
use std::fs::File;

pub(super) fn parent_flags() -> OFlags {
    let flags = OFlags::DIRECTORY | OFlags::CLOEXEC;
    #[cfg(target_os = "linux")]
    {
        flags | OFlags::PATH
    }
    #[cfg(target_vendor = "apple")]
    {
        flags | OFlags::from_bits_retain(libc::O_SEARCH.cast_unsigned())
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    {
        flags | OFlags::RDONLY
    }
}

#[cfg(target_vendor = "apple")]
pub(super) fn clear_inherited_acl(directory: &File) -> Result<(), String> {
    // chmod's ACL operation accepts the held directory on macOS, unlike pathname
    // traversal through /dev/fd/N/.; never address the replaceable staging name.
    let status = std::process::Command::new("/bin/chmod")
        .args(["-N", "/dev/fd/0"])
        .env_clear()
        .stdin(directory.try_clone().map_err(|e| e.to_string())?)
        .output()
        .map_err(|e| format!("get_url: cannot clear staging ACL: {e}"))?;
    if !status.status.success() {
        return Err("get_url: cannot clear staging ACL".into());
    }
    Ok(())
}

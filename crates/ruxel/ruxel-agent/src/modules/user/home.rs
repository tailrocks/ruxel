//! The declared home directory of an account is the agent's to create.

use std::os::unix::fs::MetadataExt;

/// The declared home directory exists and belongs to the account:
/// `usermod -d` moves the passwd field only, so the directory is the
/// agent's to create. Returns whether it changed (or would change in
/// check mode).
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
) -> Result<bool, String> {
    let dir = std::path::Path::new(home);
    if dir.is_dir() {
        return Ok(false);
    }
    if check_mode {
        return Ok(true);
    }
    let uid =
        u32::try_from(uid).map_err(|_| format!("user: home {home}: uid {uid} does not fit"))?;
    let gid =
        u32::try_from(gid).map_err(|_| format!("user: home {home}: gid {gid} does not fit"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("user: create home {home}: {e}"))?;
    let meta = std::fs::metadata(dir).map_err(|e| format!("user: own home {home}: {e}"))?;
    if meta.uid() != uid || meta.gid() != gid {
        std::os::unix::fs::chown(dir, Some(uid), Some(gid))
            .map_err(|e| format!("user: own home {home}: {e}"))?;
    }
    Ok(true)
}

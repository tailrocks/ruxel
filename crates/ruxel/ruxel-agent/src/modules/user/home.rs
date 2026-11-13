//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use std::os::unix::fs::MetadataExt;

/// The declared home path is usable before the account is touched:
/// `usermod -d` moves the passwd field even when the path is a regular
/// file, so the path is validated before any mutation — a refused apply
/// must leave the account as it was. A symlink is refused whatever it
/// targets: a home in an attacker-writable parent must never be
/// followed to a directory the account does not own. Only a missing path
/// is usable: any other lookup error — an ancestor that is a regular
/// file (`NotADirectory`), a permission refusal — propagates before any
/// mutation, for `usermod -d` would otherwise move the passwd field
/// first and leave a partially applied account behind.
pub(super) fn home_usable(home: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("user: home {home}: {error}")),
    }
}

/// The gid the home directory is created with: the requested primary
/// group's, whose `usermod -g` has already moved the account by the time
/// the directory is created — the account's old gid would own it and no
/// later apply would repair it — or the account's current one when the
/// step names no group.
pub(super) fn home_gid(group: Option<&str>, current_gid: u64) -> Result<u64, String> {
    match group {
        Some(g) => Ok(u64::from(super::super::resolve_gid(g)?)),
        None => Ok(current_gid),
    }
}

/// The declared home directory exists and belongs to the account:
/// `usermod -d` moves the passwd field only, so the directory is the
/// agent's to create — and one that exists but drifted to another uid or
/// gid is the agent's to reconcile, never a no-change early return. A
/// symlink at the path is refused and ownership changes never follow
/// one (`lchown`), so a link to an arbitrary directory can never have
/// that target handed to the account. Returns whether it changed (or
/// would change in check mode).
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
) -> Result<bool, String> {
    let dir = std::path::Path::new(home);
    let uid =
        u32::try_from(uid).map_err(|_| format!("user: home {home}: uid {uid} does not fit"))?;
    let gid =
        u32::try_from(gid).map_err(|_| format!("user: home {home}: gid {gid} does not fit"))?;
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => {
            if meta.uid() == uid && meta.gid() == gid {
                return Ok(false);
            }
            if check_mode {
                return Ok(true);
            }
            std::os::unix::fs::lchown(dir, Some(uid), Some(gid))
                .map_err(|e| format!("user: own home {home}: {e}"))?;
            Ok(true)
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if check_mode {
                return Ok(true);
            }
            std::fs::create_dir_all(dir).map_err(|e| format!("user: create home {home}: {e}"))?;
            let meta = std::fs::symlink_metadata(dir)
                .map_err(|e| format!("user: own home {home}: {e}"))?;
            if meta.uid() != uid || meta.gid() != gid {
                std::os::unix::fs::lchown(dir, Some(uid), Some(gid))
                    .map_err(|e| format!("user: own home {home}: {e}"))?;
            }
            Ok(true)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

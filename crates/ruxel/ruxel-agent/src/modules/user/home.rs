//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use std::os::unix::fs::MetadataExt;

/// The declared home path is usable before the account is touched:
/// `usermod -d` moves the passwd field even when the path is a regular
/// file, so the path is validated before any mutation — a refused apply
/// must leave the account as it was. A symlink is refused whatever it
/// targets: a home in an attacker-writable parent must never be
/// followed to a directory the account does not own, and every ancestor
/// of the path — which the lookup and the later `create_dir_all` and
/// `lchown` all traverse — is inspected the same way. Only a missing
/// path is usable: any other lookup error — an ancestor that is a
/// regular file (`NotADirectory`), a permission refusal — propagates
/// before any mutation, for `usermod -d` would otherwise move the passwd
/// field first and leave a partially applied account behind.
pub(super) fn home_usable(home: &str) -> Result<(), String> {
    ancestors_usable(home)?;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("user: home {home}: {error}")),
    }
}

/// Every ancestor of the home path, inspected without following it:
/// `create_dir_all` and `lchown` are path-based, so a symlinked
/// component would be traversed as root and the operation could create
/// or chown beneath a target the account does not own. A symlinked
/// ancestor is refused wherever a non-root principal could plant or
/// replace it, and admitted only where the parent is root's and
/// unwritable to the rest — the system links an operator places (`/var`
/// on macOS) are the operator's configuration, not the attacker's to
/// redirect. A missing component ends the walk: the rest of the path
/// does not exist yet, so nothing can traverse it.
fn ancestors_usable(home: &str) -> Result<(), String> {
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(home).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => planted_in(&prefix, home)?,
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "user: home {home}: {} is not a directory",
                    prefix.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(())
}

/// Whether a symlinked ancestor of the home path stands where a
/// non-root principal could plant or replace it: its parent is group-
/// or other-writable, or not root's. Such a link is the attacker's to
/// redirect a root traversal through; a link only root can replace is
/// system configuration the operator chose, and the path resolves
/// through it as it always did.
fn planted_in(link: &std::path::Path, home: &str) -> Result<(), String> {
    let parent = match link.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let meta = std::fs::metadata(parent)
        .map_err(|error| format!("user: home {home}: {}: {error}", parent.display()))?;
    if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        return Err(format!(
            "user: home {home}: {} is a symlink in {}, which a non-root principal can rewrite",
            link.display(),
            parent.display()
        ));
    }
    Ok(())
}

/// The home `useradd` creates for an account: the declared one, or the
/// `/home/<name>` the lowering's footprint reserves. The base directory
/// is host configuration (`useradd -D`), so the creation passes the home
/// explicitly: `useradd -m` without `-d` would put it wherever the
/// host's configured base points, and a same-stage file at the reserved
/// path would collide with account creation.
pub(super) fn claimed_home(home: Option<&str>, name: &str) -> String {
    home.map_or_else(|| format!("/home/{name}"), str::to_string)
}

/// The gid the home directory is created with: the requested primary
/// group's, whose `usermod -g` has already moved the account by the time
/// the directory is created — the account's old gid would own it and no
/// later apply would repair it — or the account's current one when the
/// step names no group. `None` is check mode's answer for a group that
/// is not on the host yet (see `planned_gid`): the plan must not require
/// the new group's live gid, and the directory's final owner is settled
/// by the primary-group change the plan already reports.
pub(super) fn home_gid(
    group: Option<&str>,
    current_gid: u64,
    check_mode: bool,
) -> Result<Option<u64>, String> {
    match group {
        None => Ok(Some(current_gid)),
        Some(g) => Ok(super::planned_gid(g, check_mode)?.map(u64::from)),
    }
}

/// The declared home directory exists and belongs to the account:
/// `usermod -d` moves the passwd field only, so the directory is the
/// agent's to create — and one that exists but drifted to another uid or
/// gid is the agent's to reconcile, never a no-change early return. A
/// symlink at the path is refused, every ancestor is inspected before
/// the path is traversed, and ownership changes never follow a link
/// (`lchown`), so neither a link at the path nor one above it can ever
/// have a target handed to the account. Returns whether it changed (or
/// would change in check mode).
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
) -> Result<bool, String> {
    ancestors_usable(home)?;
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

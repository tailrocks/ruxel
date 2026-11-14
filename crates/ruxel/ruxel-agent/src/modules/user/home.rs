//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use rustix::fs::{
    AtFlags, Gid, Mode, OFlags, Uid, fchown, fstat, mkdirat, open, openat, readlinkat, statat,
};
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

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
/// have a target handed to the account. The mutation itself runs on
/// file descriptors the walk opened without following a link: a
/// directory renamed aside after `ancestors_usable` inspected it leaves
/// the operation on the original inode, never on a replacement a
/// non-root principal planted. Returns whether it changed (or would
/// change in check mode).
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
) -> Result<bool, String> {
    ancestors_usable(home)?;
    let dir = Path::new(home);
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
            own_through(&open_home(home, false)?, uid, gid, home)?;
            Ok(true)
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if check_mode {
                return Ok(true);
            }
            own_through(&open_home(home, true)?, uid, gid, home)?;
            Ok(true)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

/// The claimed home of the account the step created: the directory is
/// reconciled with the account's final identity, and the account is
/// taken back when the reconciliation fails. A home that cannot take
/// the ownership — an immutable directory, a directory on a read-only
/// mount — fails the reconciliation, and the account the step created
/// must not stay behind in the passwd database when the task fails:
/// the host's account database is the workflow's to keep, and a
/// refused task leaves it as it was.
pub(super) fn reconcile_created_home(
    name: &str,
    home: &str,
    created: &super::PasswdEntry,
    check_mode: bool,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    match ensure_home(home, created.uid, created.gid, check_mode) {
        Ok(changed) => Ok(changed),
        Err(error) => match remove_account(name) {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!(
                "{error}; the account {name} could not be taken back: {rollback}"
            )),
        },
    }
}

/// The home directory opened through its parent: the walk opens the
/// directories of the path without following a link, and the leaf is
/// created or opened as itself — never resolved through a link a
/// concurrent rewrite plants at it, so the ownership below lands on
/// the directory the path named, or on nothing at all.
fn open_home(home: &str, create: bool) -> Result<OwnedFd, String> {
    let path = Path::new(home);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| format!("user: home {home}: no directory name"))?;
    let dir = open_chain(parent, home, create)?;
    match statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        // The leaf is judged from its own lstat, never from the open
        // that would follow it: a link planted here is refused, not
        // resolved to a target the account does not own.
        Ok(meta)
            if rustix::fs::FileType::from_raw_mode(meta.st_mode)
                == rustix::fs::FileType::Symlink =>
        {
            Err(format!("user: home {home} exists and is a symlink"))
        }
        Ok(_) => open_leaf(&dir, name, home),
        Err(rustix::io::Errno::NOENT) if create => {
            mkdirat(&dir, name, Mode::from_raw_mode(0o777))
                .map_err(|error| format!("user: create home {home}: {error}"))?;
            open_leaf(&dir, name, home)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

fn open_leaf(dir: &OwnedFd, name: &OsStr, home: &str) -> Result<OwnedFd, String> {
    openat(dir, name, walk_flags(), Mode::empty())
        .map_err(|error| format!("user: own home {home}: {error}"))
}

/// The flags every directory of the walk opens with: never following a
/// link, so a component replaced after `ancestors_usable` inspected the
/// path cannot redirect the operation to a target the account does not
/// own.
fn walk_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Every directory of `path` opened from the root down, one component
/// at a time through `openat` — creating the missing ones when
/// `create`. The walk is anchored to the inodes it opened: a directory
/// renamed aside and replaced after the walk passed it leaves the
/// operation on the original inode, and a link planted where a
/// non-root principal could rewrite it is refused rather than
/// followed. A link only root can rewrite is the operator's
/// configuration (`/var` on macOS, `/home`) — the admission
/// `planted_in` grants it — and is resolved by its target, walked
/// under the same no-follow rule.
pub(super) fn open_chain(path: &Path, home: &str, create: bool) -> Result<OwnedFd, String> {
    let mut names: Vec<OsString> = Vec::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => names.push(name.to_os_string()),
            Component::ParentDir => names.push(OsString::from("..")),
            Component::Prefix(_) => return Err(format!("user: home {home}: invalid path")),
        }
    }
    let mut current = open(
        if path.is_absolute() { "/" } else { "." },
        walk_flags(),
        Mode::empty(),
    )
    .map_err(|error| format!("user: home {home}: {error}"))?;
    let mut expansions = 0;
    let mut index = 0;
    while index < names.len() {
        let name = names[index].clone();
        match openat(&current, &name, walk_flags(), Mode::empty()) {
            Ok(fd) => {
                current = fd;
                index += 1;
            }
            Err(rustix::io::Errno::NOENT) if create => {
                mkdirat(&current, &name, Mode::from_raw_mode(0o777)).map_err(|error| {
                    format!(
                        "user: create home {home}: {}: {error}",
                        Path::new(&name).display()
                    )
                })?;
                current = openat(&current, &name, walk_flags(), Mode::empty())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                index += 1;
            }
            Err(rustix::io::Errno::NOENT) => {
                return Err(format!(
                    "user: home {home}: {} is missing",
                    Path::new(&name).display()
                ));
            }
            // A link the no-follow open refuses answers `ELOOP` on
            // Linux and `ENOTDIR` on the BSDs (a link under
            // `O_DIRECTORY`), so the component's own lstat decides
            // what the refusal was: a link is resolved under the
            // admission rule, anything else is not a directory.
            Err(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) => {
                let meta = statat(&current, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                if rustix::fs::FileType::from_raw_mode(meta.st_mode)
                    != rustix::fs::FileType::Symlink
                {
                    return Err(format!(
                        "user: home {home}: {} is not a directory",
                        Path::new(&name).display()
                    ));
                }
                // The link is admitted only where it stands: its
                // directory is root's and not rewritable, the rule
                // `planted_in` applies to the path-based walk.
                let parent =
                    fstat(&current).map_err(|error| format!("user: home {home}: {error}"))?;
                if parent.st_uid != 0 || parent.st_mode & 0o022 != 0 {
                    return Err(format!(
                        "user: home {home}: {} is a symlink in a directory a non-root principal can rewrite",
                        Path::new(&name).display()
                    ));
                }
                expansions += 1;
                if expansions > 8 {
                    return Err(format!("user: home {home}: too many symlink expansions"));
                }
                let target = readlinkat(&current, &name, Vec::new())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                let target = OsString::from_vec(target.into_bytes());
                if Path::new(&target).is_absolute() {
                    current = open("/", walk_flags(), Mode::empty())
                        .map_err(|error| format!("user: home {home}: {error}"))?;
                }
                let mut replacement: Vec<OsString> = Path::new(&target)
                    .components()
                    .filter_map(|part| match part {
                        Component::Normal(part) => Some(part.to_os_string()),
                        Component::ParentDir => Some(OsString::from("..")),
                        _ => None,
                    })
                    .collect();
                replacement.extend(names[index + 1..].iter().cloned());
                names.truncate(index);
                names.extend(replacement);
            }
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(current)
}

/// The ownership the walked directory is given, on the descriptor the
/// walk opened: the fd names the inode the path resolved to when the
/// walk opened it, so a rewrite after that point changes nothing about
/// which directory is owned.
fn own_through(fd: &OwnedFd, uid: u32, gid: u32, home: &str) -> Result<(), String> {
    let meta = fstat(fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if meta.st_uid != uid || meta.st_gid != gid {
        fchown(fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|error| format!("user: own home {home}: {error}"))?;
    }
    Ok(())
}

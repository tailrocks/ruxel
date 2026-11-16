//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use rustix::fs::{
    AtFlags, Gid, Mode, OFlags, Uid, fchmod, fchown, fstat, mkdirat, open, openat, readlinkat,
    statat,
};
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// The mode a created home directory gets: the account's own — the
/// schema exposes no home-mode field, so the mode is the agent's
/// policy (`useradd -D`'s `HOME_MODE`).
const HOME_MODE: Mode = Mode::from_raw_mode(0o750);

/// The mode a created parent of the home gets: traversable, and never
/// world-writable — a parent others may write is one they may replace.
const PARENT_MODE: Mode = Mode::from_raw_mode(0o755);

#[cfg(test)]
pub(super) use super::adoption::managed_claim;
#[cfg(test)]
pub(super) use super::adoption::operating_uid;
pub(super) use super::adoption::{
    ManagedAccount, adoption_pinned, adoption_trusted, ancestors_usable, enforce_mode, home_usable,
    symlink_target_trusted, trusted_ancestors,
};

/// The home `useradd` creates for an account: the declared one, or
/// the footprint's `/home/<name>` — passed explicitly, never left to
/// the host's configured base.
pub(super) fn claimed_home(home: Option<&str>, name: &str) -> String {
    home.map_or_else(|| format!("/home/{name}"), str::to_string)
}

/// The gid the home directory is created with: the requested primary
/// group's — `usermod -g` has already moved the account by the time
/// the directory is created — or the account's current one when the
/// step names no group. `None` is check mode's answer for a group that
/// is not on the host yet (see `planned_gid`).
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
/// agent's to create — and one that drifted to another gid is the
/// agent's to reconcile. A symlink at the path is refused, every
/// ancestor is inspected before the path is traversed, a missing home
/// is created only under trusted ancestors, and ownership changes
/// never follow a link, so no link can hand the account a target. The
/// mutation runs on descriptors the walk opened without following a
/// link. Every directory the repair creates is collected for the
/// caller: a later failure takes back exactly what the repair added.
/// The adoption admits the directory only for the managed account.
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
    managed: &ManagedAccount<'_>,
    created: &mut Vec<PathBuf>,
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
            // An existing directory is adopted, never taken blindly:
            // a directory a non-root principal may have placed must
            // never become the account's home with its contents.
            adoption_trusted(home, &meta, managed)?;
            if meta.uid() == uid && meta.gid() == gid {
                return Ok(false);
            }
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, false, created)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the path the check above inspected may have been
            // replaced in between.
            adoption_pinned(home, &fd, managed)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A missing home is created beneath its ancestors — and
            // then owned — so the creation is admitted only under
            // trusted ones, the way an adoption is, in check mode as
            // much as in apply.
            trusted_ancestors(home)?;
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, true, created)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the leaf the lookup found missing may have been
            // created in between — a directory a non-root principal
            // planted must never become the account's home with its
            // contents. A directory the creation just made is the
            // operator's own and passes.
            adoption_pinned(home, &fd, managed)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

/// The claimed home of the account the step created: reconciled, and
/// the account taken back when the reconciliation fails.
pub(super) fn reconcile_created_home(
    name: &str,
    home: &str,
    created: &super::PasswdEntry,
    check_mode: bool,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    // The created account claims the home; an oversized number matches none.
    let managed = ManagedAccount {
        name,
        uid: u32::try_from(created.uid).ok(),
    };
    let mut added = Vec::new();
    let reconciled: Result<bool, String> = (|| {
        let ownership = ensure_home(
            home,
            created.uid,
            created.gid,
            check_mode,
            &managed,
            &mut added,
        )?;
        let mode = enforce_created_mode(home, check_mode)?;
        Ok(ownership || mode)
    })();
    match reconciled {
        Ok(changed) => Ok(changed),
        Err(error) => {
            // Parents the failed repair created before it failed go
            // with the account: a retry must not inherit the partial
            // hierarchy the refused change leaves behind.
            let error = if added.is_empty() {
                error
            } else {
                match super::apply::rollback_home(home, None, &added) {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; {cleanup}"),
                }
            };
            match remove_account(name) {
                Ok(()) => Err(error),
                Err(rollback) => Err(format!(
                    "{error}; the account {name} could not be taken back: {rollback}"
                )),
            }
        }
    }
}

/// The mode of the claimed home of an account the step created: the
/// policy's, not the host's creation default — read off the directory
/// itself after the reconciliation owns it.
fn enforce_created_mode(home: &str, check_mode: bool) -> Result<bool, String> {
    let fd = open_home(home, false, &mut Vec::new())?;
    let meta = fstat(&fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if Mode::from_raw_mode(meta.st_mode & 0o777) == HOME_MODE {
        return Ok(false);
    }
    if check_mode {
        return Ok(true);
    }
    fchmod(&fd, HOME_MODE).map_err(|error| format!("user: create home {home}: {error}"))?;
    Ok(true)
}

/// The home directory opened through its parent: the walk opens the
/// directories of the path without following a link, and the leaf is
/// created or opened as itself — never resolved through a planted
/// link, so the ownership below lands where the path named.
pub(super) fn open_home(
    home: &str,
    create: bool,
    created: &mut Vec<PathBuf>,
) -> Result<OwnedFd, String> {
    let path = Path::new(home);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| format!("user: home {home}: no directory name"))?;
    let dir = open_chain(parent, home, create, created)?;
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
            mkdirat(&dir, name, HOME_MODE)
                .map_err(|error| format!("user: create home {home}: {error}"))?;
            let leaf = open_leaf(&dir, name, home)?;
            enforce_mode(&leaf, HOME_MODE, home)?;
            created.push(PathBuf::from(home));
            Ok(leaf)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

fn open_leaf(dir: &OwnedFd, name: &OsStr, home: &str) -> Result<OwnedFd, String> {
    openat(dir, name, walk_flags(), Mode::empty())
        .map_err(|error| format!("user: own home {home}: {error}"))
}

/// The flags every directory of the walk opens with: never following
/// a link, so a component replaced after the inspection cannot
/// redirect the operation to a target the account does not own.
fn walk_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// The walk's resolved position follows the component it opened:
/// `..` climbs back up, anything else descends.
fn track_resolved(resolved: &mut PathBuf, name: &OsStr) {
    if name.to_str() == Some("..") {
        resolved.pop();
    } else {
        resolved.push(name);
    }
}

/// Every directory of `path` opened from the root down through
/// `openat` — creating the missing ones when `create`, collecting
/// every one it creates so a later failure takes them back. A link a
/// non-root principal could rewrite is refused rather than followed.
pub(super) fn open_chain(
    path: &Path,
    home: &str,
    create: bool,
    created: &mut Vec<PathBuf>,
) -> Result<OwnedFd, String> {
    let mut names: Vec<OsString> = Vec::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => names.push(name.to_os_string()),
            Component::ParentDir => names.push(OsString::from("..")),
            Component::Prefix(_) => return Err(format!("user: home {home}: invalid path")),
        }
    }
    // The walk's resolved position, lexically: `..` climbs back up —
    // the directory a relative symlink target resolves against.
    let mut resolved = PathBuf::from(if path.is_absolute() { "/" } else { "." });
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
                track_resolved(&mut resolved, &name);
                current = fd;
                index += 1;
            }
            Err(rustix::io::Errno::NOENT) if create => {
                mkdirat(&current, &name, PARENT_MODE).map_err(|error| {
                    format!(
                        "user: create home {home}: {}: {error}",
                        Path::new(&name).display()
                    )
                })?;
                current = openat(&current, &name, walk_flags(), Mode::empty())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                track_resolved(&mut resolved, &name);
                created.push(resolved.clone());
                enforce_mode(&current, PARENT_MODE, home)?;
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
                let target = readlinkat(&current, &name, Vec::new())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                let target = OsString::from_vec(target.into_bytes());
                // The target's own directories are trusted the way
                // ancestors are before the walk enters any of them.
                symlink_target_trusted(&resolved, Path::new(&target), home)?;
                expansions += 1;
                if expansions > 8 {
                    return Err(format!("user: home {home}: too many symlink expansions"));
                }
                if Path::new(&target).is_absolute() {
                    current = open("/", walk_flags(), Mode::empty())
                        .map_err(|error| format!("user: home {home}: {error}"))?;
                    // The walk restarts at the root with the target's
                    // components: the resolved position restarts with
                    // it, or a later relative target would resolve
                    // against the already-walked prefix.
                    resolved = PathBuf::from("/");
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
pub(super) fn own_through(fd: &OwnedFd, uid: u32, gid: u32, home: &str) -> Result<(), String> {
    let meta = fstat(fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if meta.st_uid != uid || meta.st_gid != gid {
        fchown(fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|error| format!("user: own home {home}: {error}"))?;
    }
    Ok(())
}

/// The adoption of an existing home: the directory a non-root
/// principal may have placed is refused, the operator's own staging is
/// adopted — over scratch directories, never a host account.
#[cfg(test)]
#[path = "home/tests/adoption.rs"]
mod tests;

#[cfg(test)]
#[path = "home/tests/ancestors.rs"]
mod ancestor_tests;

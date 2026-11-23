//! The home directory opened without following a link: the walk,
//! the leaf open, and the ownership change on the open descriptor.

use rustix::fs::{
    AtFlags, Gid, Mode, OFlags, Uid, fchown, fstat, mkdirat, open, openat, readlinkat, statat,
};
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

use super::super::adoption::{enforce_mode, symlink_target_trusted};
use super::{HOME_MODE, PARENT_MODE};

/// The home directory opened through its parent: the walk opens the
/// directories of the path without following a link, and the leaf is
/// created or opened as itself — never resolved through a planted
/// link, so the ownership below lands where the path named.
pub(crate) fn open_home(
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
pub(crate) fn open_chain(
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
pub(crate) fn own_through(fd: &OwnedFd, uid: u32, gid: u32, home: &str) -> Result<(), String> {
    let meta = fstat(fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if meta.st_uid != uid || meta.st_gid != gid {
        fchown(fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|error| format!("user: own home {home}: {error}"))?;
    }
    Ok(())
}

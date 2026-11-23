//! The tree under a target: every entry is taken away through the
//! descriptor that holds it, so a name that is replaced in between is
//! never followed.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat, unlinkat};
use std::collections::HashSet;
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::path::Path;

/// The rounds a walk re-reads a directory before it declares the tree
/// gone: a writer that never stops must fail the removal instead of
/// hanging it.
const REENUMERATE_ROUNDS: usize = 8;

/// Removes the tree under the pinned directory, and the directory itself
/// is left to the caller: every entry is taken away through the
/// descriptor, so a name that is replaced in between is never followed.
/// `at` names the directory the descriptor holds, so a mount check can
/// tell mounted content from the source it is mounted from.
pub(super) fn remove_tree(dir: &OwnedFd, path: &Path, at: &Path) -> Result<(), String> {
    // The names this walk stages entries under are tracked exactly: only
    // a name this walk staged is left alone, and an entry a user named
    // with the same prefix is an entry like any other.
    let mut owned: HashSet<Vec<u8>> = HashSet::new();
    // A writer that kept a descriptor may add entries after the names are
    // read: the directory is read again until it is empty, so a child
    // that arrived late is taken too. The rounds are bounded, so a writer
    // that never stops fails the removal instead of hanging it.
    for _ in 0..REENUMERATE_ROUNDS {
        let mut pending = Vec::new();
        for name in names(dir)? {
            if owned.contains(name.to_bytes()) {
                continue;
            }
            pending.push(name);
        }
        #[cfg(test)]
        for hook in after_snapshot() {
            hook(dir, path);
        }
        if pending.is_empty() {
            return Ok(());
        }
        for name in pending {
            remove_entry(dir, &name, &mut owned, path, at)?;
        }
    }
    // The last round may have taken the last entry: read once more, and
    // declare concurrent mutation only when something is still there.
    // Without this read the removal rolls back a tree it already emptied,
    // reporting failure after deleting all of its contents.
    if names(dir)?.is_empty() {
        return Ok(());
    }
    Err(format!(
        "file: refusing to remove {}: it changed during the operation",
        path.display()
    ))
}

/// Removes one entry of a directory that is being emptied: an entry that
/// is already gone is nothing to take, and a directory is detached
/// before it is emptied.
fn remove_entry(
    dir: &OwnedFd,
    name: &CString,
    owned: &mut HashSet<Vec<u8>>,
    path: &Path,
    at: &Path,
) -> Result<(), String> {
    let meta = match statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(meta) => meta,
        // Taken between the listing and the reading: there is nothing
        // left to take, and the next round confirms it.
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let identity = (super::floor::device(&meta), meta.st_ino);
    if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
        let child = match openat(
            dir,
            name.as_c_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(child) => child,
            // Taken between the reading and the opening: there is
            // nothing left to take, and the next round confirms it.
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        take_nested(
            dir,
            name,
            &child,
            identity,
            path,
            &super::mounts::child_path(at, name),
            owned,
        )?;
    } else {
        take_plain(dir, name, identity, path)?;
    }
    Ok(())
}

/// Takes a nested directory away: the name is taken away before the
/// tree below it is emptied, so a directory that was replaced in between
/// is not the one that goes, and the tree that was read is not emptied
/// through a name that holds another directory.
pub(super) fn take_nested(
    dir: &OwnedFd,
    name: &CString,
    child: &OwnedFd,
    identity: (u64, u64),
    path: &Path,
    at: &Path,
    owned: &mut HashSet<Vec<u8>>,
) -> Result<(), String> {
    let held = super::floor::device(&rustix::fs::fstat(dir).map_err(|error| error.to_string())?);
    let opened = rustix::fs::fstat(child).map_err(|error| error.to_string())?;
    let opened_id = (super::floor::device(&opened), opened.st_ino);
    if opened_id != identity {
        // The entry is not the entry that was read: it was replaced in
        // between.
        return Err(super::attachment::replaced(path));
    }
    // The mounts are read again as the walk descends: a mount that
    // appeared after the preflight must not be followed.
    let mounts = super::mounts::mounts()?;
    if super::mounts::is_mounted(&mounts, at, opened_id) {
        // The entry is mounted content: it is the mount point itself,
        // or it lies inside the mounted tree. An entry outside every
        // mount point is the mounted tree's source, not the mount.
        return Err(format!(
            "file: refusing to remove {}: it holds the mount point {}",
            path.display(),
            at.display()
        ));
    }
    if super::floor::is_mount_point(held, super::floor::device(&opened)) {
        // The entry is on another device: it is a mount point, and the
        // tree below it belongs to somewhere else.
        return Err(format!(
            "file: refusing to remove {}: it holds the mount point {}",
            path.display(),
            name.to_string_lossy()
        ));
    }
    // The name is taken away before the tree below it is emptied: a name
    // that was replaced in between is not the tree that goes. The staged
    // name is the walk's own, and the walk leaves it alone.
    let staging = super::attachment::take_entry(dir, name, identity, path)?;
    owned.insert(staging.as_bytes().to_vec());
    let removed = (|| {
        remove_tree(child, path, at)?;
        unlinkat(dir, staging.as_c_str(), AtFlags::REMOVEDIR).map_err(|error| error.to_string())
    })();
    // The name is given back when the removal failed: a name that is
    // taken in between cannot be given back, and then the tree is
    // reported with the name it is under.
    match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(super::attachment::failed_give_back(
            super::attachment::give_back(dir, &staging, name),
            error,
        )),
    }
}

/// Takes an entry away: the name is taken away before the entry is
/// unlinked, so an entry that was replaced in between is not the one
/// that goes.
fn take_plain(
    dir: &OwnedFd,
    name: &CString,
    identity: (u64, u64),
    path: &Path,
) -> Result<(), String> {
    let staging = super::attachment::take_entry(dir, name, identity, path)?;
    let removed =
        unlinkat(dir, staging.as_c_str(), AtFlags::empty()).map_err(|error| error.to_string());
    // The name is given back when the removal failed: a name that is
    // taken in between cannot be given back, and then the entry is
    // reported with the name it is under.
    match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(super::attachment::failed_give_back(
            super::attachment::give_back(dir, &staging, name),
            error,
        )),
    }
}

/// A test hook, run after the names of a directory are read and before
/// they are taken away: a test arms it to add a child the way a writer
/// holding a descriptor would, and the walk must take it too.
#[cfg(test)]
pub(super) type AfterSnapshot = std::sync::Arc<dyn Fn(&OwnedFd, &std::path::Path) + Send + Sync>;

/// The armed hooks. A hook fires in every walk, so a test scopes its
/// hook to the directory it watches and fires it once.
#[cfg(test)]
pub(super) static AFTER_SNAPSHOT: std::sync::Mutex<Vec<AfterSnapshot>> =
    std::sync::Mutex::new(Vec::new());

/// The armed hooks.
#[cfg(test)]
fn after_snapshot() -> Vec<AfterSnapshot> {
    match AFTER_SNAPSHOT.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => Vec::new(),
    }
}

/// The names of the entries of a directory: the names are read before
/// anything is taken away, so the walk reads a directory it does not
/// change.
fn names(dir: &OwnedFd) -> Result<Vec<CString>, String> {
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

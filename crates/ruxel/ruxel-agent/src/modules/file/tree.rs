//! The tree under a target: every entry is taken away through the
//! descriptor that holds it, so a name that is replaced in between is
//! never followed.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat, unlinkat};
use std::collections::HashSet;
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Removes the tree under the pinned directory, and the directory itself
/// is left to the caller: every entry is taken away through the
/// descriptor, so a name that is replaced in between is never followed.
pub(super) fn remove_tree(dir: &OwnedFd, path: &Path) -> Result<(), String> {
    let held = std::fs::File::from(dir.try_clone().map_err(|error| error.to_string())?)
        .metadata()
        .map_err(|error| error.to_string())?;
    let held = held.dev();
    // The names this walk stages entries under are tracked exactly: only
    // a name this walk staged is left alone, and an entry a user named
    // with the same prefix is an entry like any other.
    let mut owned: HashSet<Vec<u8>> = HashSet::new();
    for name in names(dir)? {
        if owned.contains(name.to_bytes()) {
            continue;
        }
        let meta = statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| error.to_string())?;
        let identity = (super::floor::device(&meta), meta.st_ino);
        if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
            let child = openat(
                dir,
                name.as_c_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            take_nested(dir, &name, &child, identity, held, path, &mut owned)?;
        } else {
            take_plain(dir, &name, identity, path)?;
        }
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
    held: u64,
    path: &Path,
    owned: &mut HashSet<Vec<u8>>,
) -> Result<(), String> {
    let opened = rustix::fs::fstat(child).map_err(|error| error.to_string())?;
    let opened_id = (super::floor::device(&opened), opened.st_ino);
    if opened_id != identity {
        // The entry is not the entry that was read: it was replaced in
        // between.
        return Err(super::attachment::replaced(path));
    }
    // The mounts are read again as the walk descends: a mount that
    // appeared after the preflight must not be followed.
    let mounts = super::mounts::mount_identities()?;
    if super::mounts::is_mount_identity(&mounts, opened_id) {
        // The entry carries the identity of a tree the kernel mounted:
        // it is a mount point under another name.
        return Err(format!(
            "file: refusing to remove {}: it holds the mount point {}",
            path.display(),
            name.to_string_lossy()
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
        remove_tree(child, path)?;
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

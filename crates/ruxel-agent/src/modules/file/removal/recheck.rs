//! The rechecks of a removal: the mounts and the attachment are read
//! again once the directory that holds the target is held, and the
//! check walks the same boundary the run refuses at.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::super::mounts::Mount;

/// Rechecks the mounts once the directory that holds the target is held:
/// a mount installed between the first read and the open is in no table
/// the removal checked, and the held directory is where the path lands.
pub(in crate::modules::file) fn recheck_mounts(
    parent: &OwnedFd,
    path: &Path,
    mounts: &[Mount],
) -> Result<(), String> {
    // The held directory is still the one the path names: a directory
    // that was replaced in between is not the one the removal checked.
    let held = fstat(parent).map_err(|error| error.to_string())?;
    let held = (super::super::floor::device(&held), held.st_ino);
    let Some(written) = path.parent() else {
        return Err(format!(
            "file: cannot read {}: no parent directory",
            path.display()
        ));
    };
    let named = std::fs::metadata(written)
        .map_err(|error| format!("file: cannot read {}: {error}", path.display()))?;
    if (named.dev(), named.ino()) != held {
        return Err(super::super::attachment::replaced(path));
    }
    // The path through the mounts given: a covering mount the first read
    // missed lands the path in a system root.
    if let Some(root) =
        super::super::mounts::bound_root_in(path, &super::super::floor::SYSTEM_ROOTS, mounts)
    {
        return Err(format!(
            "file: refusing to remove {}: it is bound to {root}, a system root",
            path.display()
        ));
    }
    // And through the mounts of the host as they are now: a mount
    // installed after the first read lands the path somewhere else. A
    // table that cannot be read is a refusal, never an empty table.
    let fresh = super::super::mounts::mounts()
        .map_err(|error| super::super::mounts::mount_table_failed(&error))?;
    if let Some(root) =
        super::super::mounts::bound_root_in(path, &super::super::floor::SYSTEM_ROOTS, &fresh)
    {
        return Err(format!(
            "file: refusing to remove {}: it is bound to {root}, a system root",
            path.display()
        ));
    }
    Ok(())
}

/// Verifies the held directory is still the one the path names: a
/// parent that was renamed and replaced in between leaves the removal
/// working inside a detached directory, and the entry it took is not
/// the one the visible path holds.
pub(in crate::modules::file) fn verify_attached(
    parent: &OwnedFd,
    path: &Path,
) -> Result<(), String> {
    let held = fstat(parent).map_err(|error| error.to_string())?;
    let held = (super::super::floor::device(&held), held.st_ino);
    let Some(written) = path.parent() else {
        return Err(format!(
            "file: cannot read {}: no parent directory",
            path.display()
        ));
    };
    let named = std::fs::metadata(written)
        .map_err(|error| format!("file: cannot read {}: {error}", path.display()))?;
    if (named.dev(), named.ino()) != held {
        return Err(super::super::attachment::replaced(path));
    }
    Ok(())
}

/// Walks the tree the way the run walks it, and refuses where the run
/// refuses: an entry on another device is a mount point, and the tree
/// below it belongs to somewhere else. Nothing is taken here, but the
/// answer must be the run's answer.
pub(in crate::modules::file) fn check_tree(
    dir: &OwnedFd,
    path: &Path,
    at: &Path,
) -> Result<(), String> {
    check_tree_with(dir, path, at, &super::super::mounts::mounts)
}

/// Walks the tree against the mounts the listing reads, refusing where
/// the run refuses: an entry the mounts cover is mounted content even
/// on the same device — a bind mount keeps the identity of the tree it
/// is mounted from — and an entry on another device is a mount point.
///
/// The table is read again as the walk descends: a mount installed
/// after the preflight must not be walked past, the way the run does
/// not walk past it.
pub(in crate::modules::file) fn check_tree_with(
    dir: &OwnedFd,
    path: &Path,
    at: &Path,
    listing: &super::super::mounts::Listing,
) -> Result<(), String> {
    // A table that cannot be read is a refusal, and not an empty table
    // the check walks a tree with.
    let mounts = listing().map_err(|error| super::super::mounts::mount_table_failed(&error))?;
    let held = fstat(dir).map_err(|error| error.to_string())?;
    let held = super::super::floor::device(&held);
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let entry_at = super::super::mounts::child_path(at, name);
        let meta =
            statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
        let identity = (super::super::floor::device(&meta), meta.st_ino);
        if super::super::mounts::is_mounted(&mounts, &entry_at, identity) {
            return Err(format!(
                "file: refusing to remove {}: it holds the mount point {}",
                path.display(),
                entry_at.display()
            ));
        }
        if super::super::floor::is_mount_point(held, super::super::floor::device(&meta)) {
            return Err(format!(
                "file: refusing to remove {}: it holds the mount point {}",
                path.display(),
                entry_at.display()
            ));
        }
        if FileType::from_raw_mode(meta.st_mode) != FileType::Directory {
            continue;
        }
        let child = openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| error.to_string())?;
        check_tree_with(&child, path, &entry_at, listing)?;
    }
    Ok(())
}

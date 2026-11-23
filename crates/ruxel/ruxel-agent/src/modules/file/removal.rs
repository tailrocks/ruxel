//! The floor a removal never crosses, and the checks a removal makes
//! before it takes anything away.
//!
//! The removal walks the path with descriptors and never follows a
//! symlink below its first component: the directory that holds the target
//! is opened first and pinned, and the target is taken away through that
//! descriptor. A directory that is replaced in between is therefore never
//! reached, and the path is never resolved a second time.

mod parent;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat, unlinkat};
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use super::mounts::Mount;
use parent::{OpenError, open_parent};

/// Takes the path away: a removal without `recurse` takes an empty
/// directory only, with it the tree goes with the directory. Reports
/// whether the path was there.
///
/// In check mode nothing is removed: the checks a removal makes are the
/// ones the run itself makes, so both answer the same.
pub(super) fn remove(path: &Path, recurse: bool, check_mode: bool) -> Result<bool, String> {
    remove_with_listing(path, recurse, check_mode, &super::mounts::mounts)
}

/// The removal, with the mount table of the host read through the
/// listing it is given: a table that cannot be read is a refusal, and
/// not an empty table the removal walks a tree with.
pub(super) fn remove_with_listing(
    path: &Path,
    recurse: bool,
    check_mode: bool,
    listing: &super::mounts::Listing,
) -> Result<bool, String> {
    let mounts = listing().map_err(|error| super::mounts::mount_table_failed(&error))?;
    remove_with(path, recurse, check_mode, &mounts)
}

/// The removal with the mounts of the host given: a mount is a tree
/// that belongs to somewhere else, and the removal does not take it.
pub(super) fn remove_with(
    path: &Path,
    recurse: bool,
    check_mode: bool,
    mounts: &[Mount],
) -> Result<bool, String> {
    let path = absolute(path)?;
    refuse_before(&path, mounts)?;
    let (parent, name) = match open_parent(&path) {
        Ok(opened) => opened,
        // A missing ancestor means the target is not there either.
        Err(OpenError::Missing) => return Ok(false),
        Err(OpenError::Other(error)) => return Err(error),
    };
    // The mounts are read again once the parent is held: a mount
    // installed between the first read and the open changes where the
    // path lands, and the table the removal checked no longer holds.
    recheck_mounts(&parent, &path, mounts)?;
    // A bind mount aliases the directory it is mounted from: the
    // directory that holds the target has a system root's own identity,
    // and the path says nothing about it.
    refuse_bound_root(&parent, &path, &super::floor::SYSTEM_ROOTS)?;
    let meta = match statat(&parent, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(meta) => meta,
        Err(rustix::io::Errno::NOENT) => {
            // Removing something that is not there is no change.
            return Ok(false);
        }
        // Every other failure (a path that cannot be read) leaves the
        // removal unable to say what is there.
        Err(error) => {
            return Err(format!("file: cannot read {}: {error}", path.display()));
        }
    };
    let identity = (super::floor::device(&meta), meta.st_ino);
    refuse_target_mount(identity, mounts, &path)?;
    if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
        take_directory(&parent, &name, &path, recurse, check_mode, mounts)?;
    } else if !check_mode {
        // A file, and a link that points at one: the link itself is what
        // the removal takes.
        take_file(&parent, &name, identity, &path)?;
    }
    // The parent is rechecked once the work is done: a parent renamed
    // and replaced while the removal ran detaches the held descriptor,
    // and the removal must not report on a path it no longer holds.
    // Best effort by construction: a swap between this check and the
    // report races the same way every check-then-act does, but a swap
    // during the (longer) mutation itself is caught here.
    if !check_mode {
        revalidate_parent(&parent, &path)?;
    }
    Ok(true)
}

/// The path the removal works on: the absolute path, so the floor holds
/// the path that is anchored at the working directory of the agent.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let working = std::env::current_dir().map_err(|error| error.to_string())?;
    Ok(working.join(path))
}

/// The refusals a removal makes before it opens anything.
fn refuse_before(path: &Path, mounts: &[Mount]) -> Result<(), String> {
    // A removal never takes a system root, however the path reaches it.
    super::floor::refuse_system_root(path)?;
    // A tree of a system root that is mounted under another name is the
    // root's own tree: the path says nothing about where the removal
    // lands.
    if let Some(root) = super::mounts::bound_root(path, &super::floor::SYSTEM_ROOTS)? {
        return Err(format!(
            "file: refusing to remove {}: it is bound to {root}, a system root",
            path.display()
        ));
    }
    // A removal walks no dot segments: `..` would name a directory the
    // path never mentioned.
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(format!(
            "file: refusing to remove {}: a removal walks no dot segments",
            path.display()
        ));
    }
    // A target that is itself a mount point is a tree that belongs to
    // somewhere else, and the removal does not take it.
    if mounts.iter().any(|mount| mount.point == path) {
        return Err(format!(
            "file: refusing to remove {}: it is a mount point",
            path.display()
        ));
    }
    Ok(())
}

/// Refuses a target that is a mount point: the identity of the opened
/// target is the identity of a tree the kernel mounted, however the path
/// names it. The path may reach the mount point through an alias
/// (`/bin/tool` for `/usr/bin/tool`), and the name says nothing about
/// the tree.
fn refuse_target_mount(identity: (u64, u64), mounts: &[Mount], path: &Path) -> Result<(), String> {
    if super::mounts::is_mount_identity(&super::mounts::identities(mounts), identity) {
        return Err(format!(
            "file: refusing to remove {}: it is a mount point",
            path.display()
        ));
    }
    Ok(())
}

/// Takes a file away, and a link that points at one: the link itself is
/// what the removal takes. The name is taken away before the entry is
/// unlinked, so an entry that was replaced in between is not the one
/// that goes.
pub(super) fn take_file(
    parent: &OwnedFd,
    name: &CString,
    identity: (u64, u64),
    path: &Path,
) -> Result<(), String> {
    let staging = super::attachment::take_entry(parent, name, identity, path)?;
    let removed = unlinkat(parent, staging.as_c_str(), AtFlags::empty())
        .map_err(|error| format!("file: cannot remove {}: {error}", path.display()));
    // The name is given back when the removal failed: a name that is
    // taken in between cannot be given back, and then the entry is
    // reported with the name it is under.
    match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(super::attachment::failed_give_back(
            super::attachment::give_back(parent, &staging, name),
            error,
        )),
    }
}

/// Takes a directory away: an empty one without `recurse`, the tree
/// with it.
fn take_directory(
    parent: &OwnedFd,
    name: &CString,
    path: &Path,
    recurse: bool,
    check_mode: bool,
    mounts: &[Mount],
) -> Result<bool, String> {
    let child = openat(
        parent,
        name.as_c_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("file: cannot read {}: {error}", path.display()))?;
    if !recurse {
        if !directory_is_empty(&child)? {
            // A removal without `recurse` takes an empty directory only,
            // and check mode gives the same answer the run itself gives,
            // so a plan never promises a removal that cannot happen.
            return Err(super::floor::directory_not_empty());
        }
        if !check_mode {
            take_empty(parent, name, &child, path)?;
        }
        return Ok(true);
    }
    // The tree is walked before anything is taken away, so a mount
    // point in it is refused before the removal starts. The walk reads
    // the identity each entry carries, so a mount point the path reaches
    // under another name is a mount point all the same, and an entry on
    // another device is refused even when the platform writes no mount
    // table to read.
    super::mounts::preflight(&child, path, mounts)?;
    if !check_mode {
        take_tree(parent, name, &child, path)?;
    }
    Ok(true)
}

/// Takes an empty directory away: the name is taken first, so it
/// cannot be swapped while the directory goes, and the name is given
/// back when the removal fails.
fn take_empty(
    parent: &OwnedFd,
    name: &CString,
    child: &OwnedFd,
    path: &Path,
) -> Result<(), String> {
    let staging = super::attachment::take_name(parent, name, child, path)?;
    let removed = unlinkat(parent, staging.as_c_str(), AtFlags::REMOVEDIR)
        .map_err(|error| format!("file: cannot remove {}: {error}", path.display()));
    // The name is given back when the removal failed: a name that is
    // taken in between cannot be given back, and then the tree is
    // reported with the name it is under.
    match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(super::attachment::failed_give_back(
            super::attachment::give_back(parent, &staging, name),
            error,
        )),
    }
}

/// Takes a tree away: the name is taken first, so it cannot be swapped
/// while the tree goes, and the name is given back when the removal
/// fails.
fn take_tree(parent: &OwnedFd, name: &CString, child: &OwnedFd, path: &Path) -> Result<(), String> {
    let staging = super::attachment::take_name(parent, name, child, path)?;
    let removed = (|| {
        super::tree::remove_tree(child, path)?;
        unlinkat(parent, staging.as_c_str(), AtFlags::REMOVEDIR)
            .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))
    })();
    // The name is given back when the removal failed: a name that is
    // taken in between cannot be given back, and then the tree is
    // reported with the name it is under.
    match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(super::attachment::failed_give_back(
            super::attachment::give_back(parent, &staging, name),
            error,
        )),
    }
}

/// Rechecks the mounts once the directory that holds the target is held:
/// a mount installed between the first read and the open is in no table
/// the removal checked, and the held directory is where the path lands.
fn recheck_mounts(parent: &OwnedFd, path: &Path, mounts: &[Mount]) -> Result<(), String> {
    // The held directory is still the one the path names: a directory
    // that was replaced in between is not the one the removal checked.
    revalidate_parent(parent, path)?;
    // The path through the mounts given: a covering mount the first read
    // missed lands the path in a system root.
    if let Some(root) = super::mounts::bound_root_in(path, &super::floor::SYSTEM_ROOTS, mounts) {
        return Err(format!(
            "file: refusing to remove {}: it is bound to {root}, a system root",
            path.display()
        ));
    }
    // And through the mounts of the host as they are now: a mount
    // installed after the first read lands the path somewhere else. A
    // table that cannot be read is a refusal, never an empty table.
    let fresh =
        super::mounts::mounts().map_err(|error| super::mounts::mount_table_failed(&error))?;
    if let Some(root) = super::mounts::bound_root_in(path, &super::floor::SYSTEM_ROOTS, &fresh) {
        return Err(format!(
            "file: refusing to remove {}: it is bound to {root}, a system root",
            path.display()
        ));
    }
    Ok(())
}

/// Whether the directory that holds the target still holds it: the
/// removal pins the parent before it mutates anything, and a parent
/// renamed and replaced in between detaches the held descriptor. The
/// held identity is checked against the one the path names now, the same
/// way the pre-open check does.
pub(super) fn revalidate_parent(parent: &OwnedFd, path: &Path) -> Result<(), String> {
    let held = fstat(parent).map_err(|error| error.to_string())?;
    let held = (super::floor::device(&held), held.st_ino);
    let Some(written) = path.parent() else {
        return Err(format!(
            "file: cannot read {}: no parent directory",
            path.display()
        ));
    };
    let named = std::fs::metadata(written)
        .map_err(|error| format!("file: cannot read {}: {error}", path.display()))?;
    if (named.dev(), named.ino()) != held {
        return Err(super::attachment::replaced(path));
    }
    Ok(())
}

/// Refuses a removal whose directory is a system root under another name.
///
/// A bind mount (`/scratch/root` mounted from `/`) keeps the identity of
/// the directory it is mounted from, so the path says nothing about where
/// the removal lands.
pub(super) fn refuse_bound_root(
    parent: &OwnedFd,
    path: &Path,
    roots: &[&str],
) -> Result<(), String> {
    let held = std::fs::File::from(parent.try_clone().map_err(|error| error.to_string())?)
        .metadata()
        .map_err(|error| error.to_string())?;
    let held = (held.dev(), held.ino());
    let written = path.parent().map(std::path::Path::to_path_buf);
    for root in roots.iter().copied() {
        let Ok(root_meta) = std::fs::metadata(root) else {
            continue;
        };
        let root_id = (root_meta.dev(), root_meta.ino());
        let Some(written) = written.as_ref() else {
            continue;
        };
        if !aliases_root(written, held, Path::new(root), root_id) {
            continue;
        }
        return Err(format!(
            "file: refusing to remove {}: it is held by {root}, a system root",
            path.display()
        ));
    }
    Ok(())
}

/// Whether the directory that holds the target is another name for the
/// root: the same directory under a path that does not name it. A bind
/// mount (`/scratch/root` mounted from `/`) keeps the identity of the
/// directory it is mounted from, so the path says nothing about where
/// the removal lands.
pub(super) fn aliases_root(
    parent: &Path,
    parent_id: (u64, u64),
    root: &Path,
    root_id: (u64, u64),
) -> bool {
    parent_id == root_id && !names_the_same(parent, root)
}

/// Whether two paths name the same directory.
fn names_the_same(one: &Path, other: &Path) -> bool {
    if one == other {
        return true;
    }
    match (std::fs::canonicalize(one), std::fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => false,
    }
}

/// Whether a directory holds no entry.
fn directory_is_empty(dir: &OwnedFd) -> Result<bool, String> {
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if !matches!(name.to_bytes(), b"." | b"..") {
            return Ok(false);
        }
    }
    Ok(true)
}

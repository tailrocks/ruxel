//! The floor a removal never crosses, and the checks a removal makes
//! before it takes anything away.
//!
//! The removal walks the path with descriptors and never follows a
//! symlink below its first component: the directory that holds the target
//! is opened first and pinned, and the target is taken away through that
//! descriptor. A directory that is replaced in between is therefore never
//! reached, and the path is never resolved a second time.

mod parent;
pub(super) mod recheck;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat, unlinkat};
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
    recheck::recheck_mounts(&parent, &path, mounts)?;
    // A bind mount aliases the directory it is mounted from: the
    // directory that holds the target has a system root's own identity,
    // and the path says nothing about it.
    refuse_bound_root(&parent, &path, &super::floor::SYSTEM_ROOTS)?;
    remove_opened(&parent, &name, &path, recurse, check_mode, mounts)
}

/// Takes the target through the held parent directory: the directory
/// that holds the target is opened first and pinned, and the target is
/// taken away through that descriptor. Reports whether the path was
/// there.
pub(super) fn remove_opened(
    parent: &OwnedFd,
    name: &CString,
    path: &Path,
    recurse: bool,
    check_mode: bool,
    mounts: &[Mount],
) -> Result<bool, String> {
    let meta = match statat(parent, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(meta) => meta,
        Err(rustix::io::Errno::NOENT) => {
            // Removing something that is not there is no change — unless
            // the held directory is detached: then the visible path may
            // still hold a replacement, and "unchanged" names the wrong
            // tree.
            recheck::verify_attached(parent, path)?;
            return Ok(false);
        }
        // Every other failure (a path that cannot be read) leaves the
        // removal unable to say what is there.
        Err(error) => {
            return Err(format!("file: cannot read {}: {error}", path.display()));
        }
    };
    let identity = (super::floor::device(&meta), meta.st_ino);
    refuse_target_mount(identity, mounts, path)?;
    if FileType::from_raw_mode(meta.st_mode) != FileType::Directory {
        // A file, and a link that points at one: the link itself is what
        // the removal takes.
        if !check_mode {
            take_file(parent, name, identity, path)?;
            // The parent may have been renamed and replaced while the
            // entry went: then the removal took the old tree while the
            // visible path still holds its replacement.
            recheck::verify_attached(parent, path)?;
        }
        return Ok(true);
    }
    let taken = take_directory(parent, name, path, recurse, check_mode, mounts)?;
    if !check_mode {
        // As above: a detached parent makes the reported removal name
        // a tree it did not take.
        recheck::verify_attached(parent, path)?;
    }
    Ok(taken)
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

/// Refuses a target that is mounted content: the path is a mount point
/// itself, or it lies inside a mounted tree whose root identity the
/// opened target carries. The path may reach the mount point through
/// an alias (`/bin/tool` for `/usr/bin/tool`), and then the name says
/// nothing but the covering mount tells. A bind source shares the
/// mounted identity while standing outside every mount point: it is
/// the source the mount shows elsewhere, not the mount, and the
/// removal may take it.
fn refuse_target_mount(identity: (u64, u64), mounts: &[Mount], path: &Path) -> Result<(), String> {
    if super::mounts::is_mounted(mounts, path, identity) {
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
    // under another name is a mount point all the same.
    if !mounts.is_empty() {
        super::mounts::preflight(&child, path, mounts)?;
    }
    if check_mode {
        // Without a mount table the preflight above is skipped, and the
        // run still refuses at the first device boundary it crosses. The
        // check walks the same boundary, so the plan cannot approve what
        // the run cannot take.
        if recurse {
            recheck::check_tree(&child, path, path)?;
        }
        return Ok(true);
    }
    take_tree(parent, name, &child, path)?;
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
        super::tree::remove_tree(child, path, path)?;
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

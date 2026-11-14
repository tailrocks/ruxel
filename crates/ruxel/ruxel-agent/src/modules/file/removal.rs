//! The floor a removal never crosses, and the checks a removal makes
//! before it takes anything away.
//!
//! The removal walks the path with descriptors and never follows a
//! symlink below its first component: the directory that holds the target
//! is opened first and pinned, and the target is taken away through that
//! descriptor. A directory that is replaced in between is therefore never
//! reached, and the path is never resolved a second time.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, open, openat, statat, unlinkat};
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// Takes the path away: a removal without `recurse` takes an empty
/// directory only, with it the tree goes with the directory. Reports
/// whether the path was there.
///
/// In check mode nothing is removed: the checks a removal makes are the
/// ones the run itself makes, so both answer the same.
pub(super) fn remove(path: &Path, recurse: bool, check_mode: bool) -> Result<bool, String> {
    // The removal works on the absolute path: a relative path is anchored
    // at the working directory of the agent, and the floor has to hold the
    // path that is anchored there.
    let working = std::env::current_dir().map_err(|error| error.to_string())?;
    let path = &if path.is_absolute() {
        path.to_path_buf()
    } else {
        working.join(path)
    };
    // A removal never takes a system root, however the path reaches it.
    super::floor::refuse_system_root(path)?;
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
    let (parent, name) = match open_parent(path) {
        Ok(opened) => opened,
        // A missing ancestor means the target is not there either.
        Err(OpenError::Missing) => return Ok(false),
        Err(OpenError::Other(error)) => return Err(error),
    };
    // A bind mount aliases the directory it is mounted from: the
    // directory that holds the target has a system root's own identity,
    // and the path says nothing about it.
    refuse_bound_root(&parent, path, &super::floor::SYSTEM_ROOTS)?;
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
    if FileType::from_raw_mode(meta.st_mode) != FileType::Directory {
        // A file, and a link that points at one: the link itself is what
        // the removal takes.
        if !check_mode {
            unlinkat(&parent, name.as_c_str(), AtFlags::empty())
                .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))?;
        }
        return Ok(true);
    }
    let child = openat(
        &parent,
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
            unlinkat(&parent, name.as_c_str(), AtFlags::REMOVEDIR)
                .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))?;
        }
        return Ok(true);
    }
    if recurse {
        // The tree is walked before anything is taken away, so a mount
        // point in it is refused before the removal starts.
        let mounts = mount_points();
        if !mounts.is_empty() {
            preflight(&child, path, &mounts)?;
        }
    }
    if !check_mode {
        remove_tree(&child, path)?;
        unlinkat(&parent, name.as_c_str(), AtFlags::REMOVEDIR)
            .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))?;
    }
    Ok(true)
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
/// directory it is mounted from, so the path says nothing about where the
/// removal lands.
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

/// The mount points of the host, as the kernel lists them. A mount point
/// is a tree that belongs to somewhere else, and a removal does not take
/// it.
pub(super) fn mount_points() -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read("/proc/self/mountinfo") else {
        return Vec::new();
    };
    let mut points = Vec::new();
    for line in listing.split(|byte| *byte == b'\n') {
        // The mount point is the fifth field of the line.
        let mut fields = line.split(|byte| *byte == b' ');
        let mut point = None;
        for _ in 0..5 {
            match fields.next() {
                Some(field) => point = Some(field),
                None => break,
            }
        }
        let Some(point) = point else {
            continue;
        };
        points.push(PathBuf::from(std::ffi::OsString::from_vec(
            unescape_mount_bytes(point),
        )));
    }
    points
}

/// The mount path as the kernel writes it: the characters that need it
/// are written as octal escapes. The path is read as bytes, because a
/// mount point may hold a name that is not text.
pub(super) fn unescape_mount_bytes(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < path.len() {
        if path[index] != b'\\' {
            out.push(path[index]);
            index += 1;
            continue;
        }
        let mut octal = String::new();
        for byte in path.iter().skip(index + 1).take(3) {
            octal.push(char::from(*byte));
        }
        if let Ok(byte) = u8::from_str_radix(&octal, 8) {
            out.push(byte);
            index += 4;
        } else {
            out.push(b'\\');
            index += 1;
        }
    }
    out
}

/// Walks the tree without touching it, and refuses a mount point in it.
fn preflight(dir: &OwnedFd, path: &Path, mounts: &[PathBuf]) -> Result<(), String> {
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let child_path = path.join(name.to_string_lossy().as_ref());
        if mounts.iter().any(|mount| mount == &child_path) {
            return Err(format!(
                "file: refusing to remove {}: it holds the mount point {}",
                path.display(),
                child_path.display()
            ));
        }
        let meta =
            statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
        if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
            let child = openat(
                dir,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            preflight(&child, &child_path, mounts)?;
        }
    }
    Ok(())
}

/// Removes the tree under the pinned directory, and the directory itself
/// is left to the caller: every entry is taken away through the
/// descriptor, so a name that is replaced in between is never followed.
fn remove_tree(dir: &OwnedFd, path: &Path) -> Result<(), String> {
    let held = std::fs::File::from(dir.try_clone().map_err(|error| error.to_string())?)
        .metadata()
        .map_err(|error| error.to_string())?;
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let meta =
            statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
        if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
            let child = openat(
                dir,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            let entry = std::fs::File::from(child.try_clone().map_err(|error| error.to_string())?)
                .metadata()
                .map_err(|error| error.to_string())?;
            if super::floor::is_mount_point(held.dev(), entry.dev()) {
                // The entry is on another device: it is a mount point,
                // and the tree below it belongs to somewhere else.
                return Err(format!(
                    "file: refusing to remove {}: it holds the mount point {}",
                    path.display(),
                    name.to_string_lossy()
                ));
            }
            remove_tree(&child, path)?;
            unlinkat(dir, name, AtFlags::REMOVEDIR).map_err(|error| error.to_string())?;
        } else {
            unlinkat(dir, name, AtFlags::empty()).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Opens the directory that holds the path, and the name of the entry in
/// it. The walk follows the first component, which is where the system's
/// own layout lives (`/var` -> `/private/var`, `/bin` -> `/usr/bin`), and
/// no symlink below it.
/// The reason a walk to the parent of a path failed.
#[derive(Debug)]
pub(super) enum OpenError {
    /// A component of the path is not there.
    Missing,
    /// The walk failed for any other reason.
    Other(String),
}

pub(super) fn open_parent(path: &Path) -> Result<(OwnedFd, CString), OpenError> {
    let name = path.file_name().ok_or_else(|| {
        OpenError::Other(format!("file: {} does not name an entry", path.display()))
    })?;
    let name = CString::new(name.as_bytes()).map_err(|_| {
        OpenError::Other(format!(
            "file: {} names an entry with a zero byte",
            path.display()
        ))
    })?;
    let mut current = if path.is_absolute() {
        open("/", directory_flags(), Mode::empty())
    } else {
        open(".", directory_flags(), Mode::empty())
    }
    .map_err(|error| OpenError::Other(format!("file: cannot read {}: {error}", path.display())))?;
    let parts: Vec<_> = path
        .components()
        .filter(|part| {
            !matches!(
                part,
                Component::RootDir | Component::CurDir | Component::Prefix(_)
            )
        })
        .collect();
    // The last component is the entry itself; the rest is the directory
    // that holds it.
    let parents = parts.len().saturating_sub(1);
    for (index, part) in parts.iter().take(parents).enumerate() {
        let part = match part {
            Component::Normal(part) => part,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            _ => continue,
        };
        // The first component may be the system's own alias (`/var` ->
        // `/private/var`, `/bin` -> `/usr/bin`): it is followed only when
        // it points at a directory of the same name, so a link that
        // points somewhere else (`/scratch` -> `/`) is refused.
        let flags = if index == 0 {
            if super::floor::is_alias(&current, part) {
                directory_flags()
            } else {
                directory_flags() | OFlags::NOFOLLOW
            }
        } else {
            directory_flags() | OFlags::NOFOLLOW
        };
        current = openat(&current, part, flags, Mode::empty()).map_err(|error| {
            if error == rustix::io::Errno::NOENT {
                return OpenError::Missing;
            }
            OpenError::Other(format!("file: cannot read {}: {error}", path.display()))
        })?;
    }
    Ok((current, name))
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

/// The flags a directory is opened with.
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

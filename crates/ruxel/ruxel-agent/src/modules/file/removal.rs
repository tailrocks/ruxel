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
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// Takes the path away: a removal without `recurse` takes an empty
/// directory only, with it the tree goes with the directory. Reports
/// whether the path was there.
///
/// In check mode nothing is removed: the checks a removal makes are the
/// ones the run itself makes, so both answer the same.
pub(super) fn remove(path: &Path, recurse: bool, check_mode: bool) -> Result<bool, String> {
    // A removal never takes a system root, however the path reaches it.
    refuse_system_root(path)?;
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
    let (parent, name) = open_parent(path)?;
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
    .map_err(|error| format!("file: cannot open {}: {error}", path.display()))?;
    if !recurse {
        if !directory_is_empty(&child)? {
            // A removal without `recurse` takes an empty directory only,
            // and check mode gives the same answer the run itself gives,
            // so a plan never promises a removal that cannot happen.
            return Err(directory_not_empty());
        }
        if !check_mode {
            unlinkat(&parent, name.as_c_str(), AtFlags::REMOVEDIR)
                .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))?;
        }
        return Ok(true);
    }
    if !check_mode {
        remove_tree(&child)?;
        unlinkat(&parent, name.as_c_str(), AtFlags::REMOVEDIR)
            .map_err(|error| format!("file: cannot remove {}: {error}", path.display()))?;
    }
    Ok(true)
}

/// Removes the tree under the pinned directory, and the directory itself
/// is left to the caller: every entry is taken away through the
/// descriptor, so a name that is replaced in between is never followed.
fn remove_tree(dir: &OwnedFd) -> Result<(), String> {
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
            remove_tree(&child)?;
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
fn open_parent(path: &Path) -> Result<(OwnedFd, CString), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("file: {} does not name an entry", path.display()))?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| format!("file: {} names an entry with a zero byte", path.display()))?;
    let mut current = if path.is_absolute() {
        open("/", directory_flags(), Mode::empty())
    } else {
        open(".", directory_flags(), Mode::empty())
    }
    .map_err(|error| format!("file: cannot open {}: {error}", path.display()))?;
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
        let flags = if index == 0 {
            directory_flags()
        } else {
            directory_flags() | OFlags::NOFOLLOW
        };
        current = openat(&current, part, flags, Mode::empty())
            .map_err(|error| format!("file: cannot open {}: {error}", path.display()))?;
    }
    Ok((current, name))
}

/// The flags a directory is opened with.
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

/// The roots a removal never takes: the filesystem root and the
/// directories the system itself lives in.
const SYSTEM_ROOTS: [&str; 10] = [
    "/", "/etc", "/var", "/root", "/home", "/usr", "/boot", "/dev", "/proc", "/sys",
];

/// A removal never takes a system root, however the path reaches it.
///
/// The plan holds a removal against the same floor, but the plan reads the
/// path as it is written: a symlinked ancestor (`/tmp/root` -> `/`) carries
/// the removal of `/tmp/root/etc` into the real `/etc`. The ancestors of
/// the path are therefore resolved, and where they land is held against
/// the floor. The last component keeps the name it was given: a symlink
/// that points at a system root is a link, and taking a link takes
/// nothing.
pub(super) fn refuse_system_root(path: &Path) -> Result<(), String> {
    let written = written_path(path);
    if SYSTEM_ROOTS.contains(&written.as_str()) {
        return Err(format!(
            "file: refusing to remove {written}: a removal never takes a system root"
        ));
    }
    // The system's own layout (`/var` -> `/private/var`, `/bin` ->
    // `/usr/bin`) is a first component and is not a crossing.
    if !crosses_symlink(path) {
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let written_parent = parent.to_string_lossy().to_string();
    let Ok(resolved) = std::fs::canonicalize(parent) else {
        return Ok(());
    };
    let resolved = resolved.to_string_lossy().to_string();
    if resolved == "/" {
        return Err(format!(
            "file: refusing to remove {written}: it reaches the filesystem root"
        ));
    }
    for root in SYSTEM_ROOTS {
        // A root is held by the name it was written under and by the name
        // it resolves to: `/var` and `/private/var` are one root.
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
        let canonical = canonical.to_string_lossy().to_string();
        let roots = [root.to_string(), canonical];
        let reached = roots.iter().any(|root| inside(&resolved, root));
        if !reached {
            continue;
        }
        // The path was already inside the root as it was written: the
        // resolution did not take it there.
        let already = roots.iter().any(|root| inside(&written_parent, root));
        if already {
            continue;
        }
        return Err(format!(
            "file: refusing to remove {written}: it reaches {resolved}, inside the system root {root}"
        ));
    }
    Ok(())
}

/// Whether a path is a root or lies inside it.
fn inside(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

/// The path as it was written, without a trailing separator.
fn written_path(path: &Path) -> String {
    let written = path.to_string_lossy().to_string();
    let trimmed = written.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Whether the path crosses a symlink below its first component.
fn crosses_symlink(path: &Path) -> bool {
    let mut current = PathBuf::new();
    let mut first = true;
    for component in path.components() {
        match component {
            Component::RootDir => current.push("/"),
            Component::CurDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                current.push("..");
                first = false;
            }
            Component::Normal(name) => {
                current.push(name);
                if !first && is_symlink(&current) {
                    return true;
                }
                first = false;
            }
        }
    }
    false
}

/// Whether a path is a symlink.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
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

/// The error a removal of a nonempty directory gives: the message the run
/// itself produces, so check mode and the run agree.
fn directory_not_empty() -> String {
    std::io::Error::from_raw_os_error(rustix::io::Errno::NOTEMPTY.raw_os_error()).to_string()
}

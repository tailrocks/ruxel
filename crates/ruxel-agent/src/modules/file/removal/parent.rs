//! The directory that holds the target: the walk to it opens every
//! component by itself, and it follows the system's own layout and
//! nothing else.

use rustix::fs::{Mode, OFlags, open, openat};
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

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
            if super::super::floor::is_alias(&current, part) {
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

/// The flags a directory is opened with.
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

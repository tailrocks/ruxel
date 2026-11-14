//! The mount points of the host: a tree that belongs to somewhere else
//! is not the removal's to take.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

/// The mount points of the host, as the kernel lists them. A mount point
/// is a tree that belongs to somewhere else, and a removal does not take
/// it.
pub(super) fn mount_points() -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read("/proc/self/mountinfo") else {
        return Vec::new();
    };
    let mut points = Vec::new();
    for line in listing.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
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
pub(super) fn preflight(dir: &OwnedFd, path: &Path, mounts: &[PathBuf]) -> Result<(), String> {
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

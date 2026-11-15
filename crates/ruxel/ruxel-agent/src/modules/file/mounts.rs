//! The mount points of the host: a tree that belongs to somewhere else
//! is not the removal's to take.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A mount of the host: the tree that is mounted, and where it is
/// mounted. A mount keeps the identity of the tree it is mounted from,
/// so the path says nothing about where a removal lands.
#[derive(Debug)]
pub(super) struct Mount {
    /// The tree that is mounted, as the path it has in the filesystem
    /// it comes from.
    pub(super) root: PathBuf,
    /// The path the tree is mounted at.
    pub(super) point: PathBuf,
    /// The device and the inode of the tree that is mounted: the
    /// identity a bind mount keeps.
    pub(super) identity: (u64, u64),
}

/// The mounts of the host, as the kernel lists them.
///
/// The listing is read from the source the platform writes it in. A
/// listing that cannot be read is not an empty listing: without it the
/// removal cannot tell a mount point from an ordinary directory, and it
/// refuses rather than walk a tree it cannot see the mounts of.
pub(super) fn mounts() -> Result<Vec<Mount>, String> {
    mounts_from(listing())
}

/// The mount table of the host, as the removal reads it: the table is
/// read through this, so that a table that cannot be read is a refusal
/// and never an empty table.
pub(super) type Listing = dyn Fn() -> Result<Vec<Mount>, String>;

/// The mounts of the host, with the listing of the host given: a
/// listing that cannot be read is a refusal, never an empty listing.
pub(super) fn mounts_from(listing: Result<Vec<u8>, String>) -> Result<Vec<Mount>, String> {
    let listing = listing.map_err(|error| mount_table_failed(&error))?;
    Ok(parse(&listing))
}

/// The failure of reading the mount table of the host: the table holds
/// the mount points, and without it the removal cannot tell a mount
/// point from an ordinary directory.
pub(super) fn mount_table_failed(error: &str) -> String {
    format!("file: cannot read the mount points of the host: {error}")
}

/// The listing of the host, as the platform writes it.
///
/// Linux writes the mount table in `/proc/self/mountinfo`. A host
/// without a procfs — a container, a chroot — has none, and the
/// removal cannot see the mounts of the host: it refuses.
#[cfg(target_os = "linux")]
fn listing() -> Result<Vec<u8>, String> {
    std::fs::read("/proc/self/mountinfo").map_err(|error| error.to_string())
}

/// The platform writes no mount table to read: it mounts no tree under
/// another name, and a mount point is an entry of another device, which
/// the walk holds.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the listing of the host is read on the platform that writes one"
)]
fn listing() -> Result<Vec<u8>, String> {
    Ok(Vec::new())
}

/// The mounts of the host, as the listing of the host lists them.
fn parse(listing: &[u8]) -> Vec<Mount> {
    let mut mounts = Vec::new();
    for line in listing.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        // The listing holds the id of the mount, the id of its parent,
        // the device, the tree that is mounted and the path it is
        // mounted at.
        let mut fields = line.split(|byte| *byte == b' ');
        let mut root = None;
        let mut point = None;
        for index in 0..5 {
            let Some(field) = fields.next() else {
                break;
            };
            match index {
                3 => root = Some(field),
                4 => point = Some(field),
                _ => {}
            }
        }
        let (Some(root), Some(point)) = (root, point) else {
            continue;
        };
        let root = PathBuf::from(std::ffi::OsString::from_vec(unescape_mount_bytes(root)));
        let point = PathBuf::from(std::ffi::OsString::from_vec(unescape_mount_bytes(point)));
        let identity = std::fs::metadata(&point).map_or((0, 0), |meta| (meta.dev(), meta.ino()));
        mounts.push(Mount {
            root,
            point,
            identity,
        });
    }
    mounts
}

/// The identity of every mount point of the host: the device and the
/// inode of the tree that is mounted. A bind mount keeps the identity
/// of the tree it is mounted from, so an entry with that identity is a
/// mount point under another name.
pub(super) fn mount_identities() -> Result<Vec<(u64, u64)>, String> {
    Ok(identities(&mounts()?))
}

/// The identity of every mount of the list: the device and the inode of
/// the tree that is mounted.
pub(super) fn identities(mounts: &[Mount]) -> Vec<(u64, u64)> {
    mounts
        .iter()
        .map(|mount| mount.identity)
        .filter(|identity| *identity != (0, 0))
        .collect()
}

/// Whether an entry of a tree is a mount point: it carries the
/// identity of a tree the kernel mounted.
pub(super) fn is_mount_identity(mounts: &[(u64, u64)], identity: (u64, u64)) -> bool {
    mounts.contains(&identity)
}

/// The system root a path lands in, when the path reaches it through a
/// mount: a tree of a root that is mounted under another name is the
/// root's own tree, and a removal that reaches it takes the root's
/// files.
pub(super) fn bound_root(path: &Path, roots: &[&str]) -> Result<Option<String>, String> {
    Ok(bound_root_in(
        path,
        roots,
        &mounts().map_err(|error| mount_table_failed(&error))?,
    ))
}

/// The system root a path lands in, with the mounts of the host given.
pub(super) fn bound_root_in(path: &Path, roots: &[&str], mounts: &[Mount]) -> Option<String> {
    // The mount that covers the path the deepest is the one that names
    // where the path lands.
    let mut covering: Option<&Mount> = None;
    for mount in mounts {
        if !path.starts_with(&mount.point) {
            continue;
        }
        let deeper = covering.is_some_and(|other| {
            other.point.components().count() >= mount.point.components().count()
        });
        if !deeper {
            covering = Some(mount);
        }
    }
    let mount = covering?;
    let relative = path.strip_prefix(&mount.point).ok()?;
    let landed = mount.root.join(relative);
    // The mount took the path nowhere: the path lands where it was
    // written, and the mount is not an alias of anything.
    if landed == path {
        return None;
    }
    for root in roots {
        // The mount does not carry the tree of the root: the path lands
        // in a tree of its own.
        if !carries(&mount.root, Path::new(root)) {
            continue;
        }
        // The root is on another device: the mount does not carry its
        // tree, and the path lands in a filesystem of its own.
        let Ok(meta) = std::fs::metadata(root) else {
            continue;
        };
        if meta.dev() != mount.identity.0 {
            continue;
        }
        // The path was already inside the root as it was written: the
        // mount did not take it there. The filesystem root holds only
        // itself: every absolute path starts with it, and the test
        // would excuse every alias.
        if inside(path, Path::new(root)) {
            continue;
        }
        return Some((*root).to_string());
    }
    None
}

/// Whether a mount carries the tree of a root: the tree that is mounted
/// is the root, or a directory below it.
fn carries(mounted: &Path, root: &Path) -> bool {
    mounted == root || inside(mounted, root)
}

/// Whether a path lies inside a root: a root holds itself and the tree
/// below it, but the filesystem root holds only itself.
fn inside(path: &Path, root: &Path) -> bool {
    if root.parent().is_none() {
        // The filesystem root: every absolute path starts with it, and
        // it holds nothing but itself.
        return path == root;
    }
    path.starts_with(root)
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

/// The path of an entry below a directory: the name of an entry is not
/// necessarily text, so it is not made text on the way.
pub(super) fn child_path(dir: &Path, name: &std::ffi::CStr) -> PathBuf {
    dir.join(std::ffi::OsString::from_vec(name.to_bytes().to_vec()))
}

/// Walks the tree without touching it, and refuses a mount point in it.
/// A mount point is the path the kernel lists, and the identity the
/// mounted tree carries: the path says nothing when the tree is reached
/// under another name (`/bin/tool` for `/usr/bin/tool`), and only the
/// identity the entry carries tells then.
pub(super) fn preflight(dir: &OwnedFd, path: &Path, mounts: &[Mount]) -> Result<(), String> {
    let identities = identities(mounts);
    let entries = rustix::fs::Dir::read_from(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        // The path of the entry keeps the bytes of its name: a name
        // that is not text is the name the mount has, and a path made
        // of text would miss the mount.
        let child_path = child_path(path, name);
        if mounts.iter().any(|mount| mount.point == child_path) {
            return Err(format!(
                "file: refusing to remove {}: it holds the mount point {}",
                path.display(),
                child_path.display()
            ));
        }
        let meta =
            statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
        // A bind mount keeps the identity of the tree it is mounted
        // from: the entry carries the identity of a mount point under
        // another name, and the spelling of the path says nothing.
        if is_mount_identity(&identities, (super::floor::device(&meta), meta.st_ino)) {
            return Err(format!(
                "file: refusing to remove {}: it holds the mount point {}",
                path.display(),
                child_path.display()
            ));
        }
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

//! The attachment of a target to its name: the removal holds the
//! directory it opened, and the name it takes away must be that same
//! directory.

use rustix::fs::{AtFlags, RenameFlags, renameat_with, statat};
use rustix::io::Errno;
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// The counter that keeps the staging names of the process apart.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Takes the name of the target away: the entry is renamed to a name
/// of its own inside the directory that holds it, so the name the
/// removal was asked for cannot be swapped while the target goes. The
/// rename is checked against the directory that was opened: a name
/// that was replaced in between is put back, and the removal refuses.
pub(super) fn take_name(
    parent: &OwnedFd,
    name: &CString,
    opened: &OwnedFd,
    path: &Path,
) -> Result<CString, String> {
    for _ in 0..128 {
        let staging = CString::new(format!(
            ".ruxel-remove-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| format!("file: cannot name the staging entry of {}", path.display()))?;
        match renameat_with(
            parent,
            name.as_c_str(),
            parent,
            staging.as_c_str(),
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {}
            Err(Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!("file: cannot remove {}: {error}", path.display()));
            }
        }
        // The name that was taken away must be the directory that was
        // opened: a name that was replaced in between is put back.
        match holds_name(parent, staging.as_c_str(), opened) {
            Ok(true) => return Ok(staging),
            Ok(false) => {
                give_back(parent, &staging, name);
                return Err(format!(
                    "file: refusing to remove {}: it was replaced during the operation",
                    path.display()
                ));
            }
            Err(error) => {
                give_back(parent, &staging, name);
                return Err(format!("file: cannot remove {}: {error}", path.display()));
            }
        }
    }
    Err(format!(
        "file: cannot reserve a staging name for {}",
        path.display()
    ))
}

/// Whether the name still holds the directory that was opened.
pub(super) fn holds_name(
    parent: &OwnedFd,
    name: &std::ffi::CStr,
    opened: &OwnedFd,
) -> Result<bool, String> {
    let meta =
        statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
    let opened = rustix::fs::fstat(opened).map_err(|error| error.to_string())?;
    Ok(
        (super::floor::device(&meta), meta.st_ino)
            == (super::floor::device(&opened), opened.st_ino),
    )
}

/// Gives the name back: the entry that was taken away goes back to the
/// name it had.
pub(super) fn give_back(parent: &OwnedFd, staging: &CString, name: &CString) {
    let _ = renameat_with(
        parent,
        staging.as_c_str(),
        parent,
        name.as_c_str(),
        RenameFlags::NOREPLACE,
    );
}

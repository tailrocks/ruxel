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

/// The prefix of the names the process stages entries under: the names
/// are its own, and the walk of a tree leaves them alone.
const PREFIX: &str = ".ruxel-remove-";

/// Whether a name is one the process stages entries under: the walk of
/// a tree skips them, or it takes an entry of its own.
pub(super) fn is_staging(name: &std::ffi::CStr) -> bool {
    name.to_bytes().starts_with(PREFIX.as_bytes())
}

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
    let staging = stage(parent, name, path)?;
    match holds_name(parent, staging.as_c_str(), opened) {
        Ok(true) => Ok(staging),
        Ok(false) => Err(failed_give_back(
            give_back(parent, &staging, name),
            replaced(path),
        )),
        Err(error) => Err(failed_give_back(
            give_back(parent, &staging, name),
            format!("file: cannot remove {}: {error}", path.display()),
        )),
    }
}

/// Takes the name of an entry away and checks that it still holds the
/// entry that was read: the identity that was read is the identity the
/// staged name must carry. An entry that was replaced in between is put
/// back, and the removal refuses.
pub(super) fn take_entry(
    parent: &OwnedFd,
    name: &CString,
    identity: (u64, u64),
    path: &Path,
) -> Result<CString, String> {
    let staging = stage(parent, name, path)?;
    let held = statat(parent, staging.as_c_str(), AtFlags::SYMLINK_NOFOLLOW);
    match held {
        Ok(meta) if (super::floor::device(&meta), meta.st_ino) == identity => Ok(staging),
        Ok(_) => Err(failed_give_back(
            give_back(parent, &staging, name),
            replaced(path),
        )),
        Err(error) => Err(failed_give_back(
            give_back(parent, &staging, name),
            format!("file: cannot remove {}: {error}", path.display()),
        )),
    }
}

/// Moves the name of an entry to a name of its own inside the directory
/// that holds it, so the name the removal was asked for cannot be
/// swapped while the entry goes.
fn stage(parent: &OwnedFd, name: &CString, path: &Path) -> Result<CString, String> {
    for _ in 0..128 {
        let staging = CString::new(format!(
            "{PREFIX}{}-{}",
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
            Ok(()) => return Ok(staging),
            // The name of the staging entry is taken: another name is
            // tried.
            Err(Errno::EXIST) => {}
            Err(error) => {
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
pub(super) fn give_back(parent: &OwnedFd, staging: &CString, name: &CString) -> Result<(), String> {
    renameat_with(
        parent,
        staging.as_c_str(),
        parent,
        name.as_c_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        format!(
            "file: cannot give {} back: {error}; the tree is at {}",
            name.to_string_lossy(),
            staging.to_string_lossy()
        )
    })
}

/// The failure of a removal with the failure of the rollback that gave
/// the name back: a tree that is not given back is reported with the
/// name it is under.
pub(super) fn failed_give_back(given: Result<(), String>, error: String) -> String {
    match given {
        Ok(()) => error,
        Err(rollback) => format!("{error}; {rollback}"),
    }
}

/// The refusal of a removal whose target was replaced in between.
pub(super) fn replaced(path: &Path) -> String {
    format!(
        "file: refusing to remove {}: it was replaced during the operation",
        path.display()
    )
}

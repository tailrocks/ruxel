//! The floor a removal never crosses, and the checks a removal makes
//! before it takes anything away.

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
    if path.symlink_metadata().is_err() {
        // Removing something that is not there is no change.
        return Ok(false);
    }
    let directory = path.is_dir() && !path.is_symlink();
    if directory && !recurse && !directory_is_empty(path)? {
        // A removal without `recurse` takes an empty directory only, and
        // check mode gives the same answer the run itself gives, so a
        // plan never promises a removal that cannot happen.
        return Err(directory_not_empty());
    }
    if !check_mode {
        if directory {
            if recurse {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_dir(path).map_err(|e| e.to_string())?;
            }
        } else {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(true)
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
/// the removal of `/tmp/root/etc` into the real `/etc`. The path is
/// therefore resolved, and the result is held against the floor.
pub(super) fn refuse_system_root(path: &Path) -> Result<(), String> {
    let written = written_path(path);
    if SYSTEM_ROOTS.contains(&written.as_str()) {
        return Err(format!(
            "file: refusing to remove {written}: a removal never takes a system root"
        ));
    }
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return Ok(());
    };
    let resolved = resolved.to_string_lossy().to_string();
    for root in SYSTEM_ROOTS {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
        let canonical = canonical.to_string_lossy().to_string();
        if resolved == canonical {
            return Err(format!(
                "file: refusing to remove {written}: it resolves to {canonical}, a system root"
            ));
        }
        // A path that crosses a symlink lands where the link points, and
        // the contents of a system root are not the plan's to take. The
        // system's own layout (`/var` -> `/private/var`, `/bin` ->
        // `/usr/bin`) is a first component and is not a crossing.
        if canonical != "/"
            && resolved.starts_with(&format!("{canonical}/"))
            && crosses_symlink(path)
        {
            return Err(format!(
                "file: refusing to remove {written}: it resolves to {resolved}, inside the system root {canonical}"
            ));
        }
    }
    Ok(())
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
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
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
pub(super) fn directory_is_empty(path: &Path) -> Result<bool, String> {
    let mut entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    Ok(entries.next().is_none())
}

/// The error a removal of a nonempty directory gives: the message the run
/// itself produces, so check mode and the run agree.
pub(super) fn directory_not_empty() -> String {
    std::io::Error::from_raw_os_error(rustix::io::Errno::NOTEMPTY.raw_os_error()).to_string()
}

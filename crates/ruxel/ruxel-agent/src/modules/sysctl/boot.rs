//! The boot surface of sysctl: the files `sysctl --system` reads, the
//! order they load in, and the shadow and override probes over them.
//!
//! procps reads five system directories and the legacy file. It loads
//! one file per name — the copy in the highest-precedence directory
//! that holds it — and the names load together, sorted, with
//! `/etc/sysctl.conf` after all of them.

mod analysis;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(super) use analysis::boot_override;

/// The recognized system directories, by precedence: the higher rank
/// wins a file name shared between them. procps reads them in this
/// order (`sysctl.conf(5)`): `/etc/sysctl.d`, `/run/sysctl.d`,
/// `/usr/local/lib/sysctl.d`, `/usr/lib/sysctl.d`, `/lib/sysctl.d`.
pub(super) const DIRS: [(&str, u8); 5] = [
    ("/etc/sysctl.d/", 4),
    ("/run/sysctl.d/", 3),
    ("/usr/local/lib/sysctl.d/", 2),
    ("/usr/lib/sysctl.d/", 1),
    ("/lib/sysctl.d/", 0),
];

/// The legacy file, loaded after every directory file.
pub(super) const LEGACY: &str = "/etc/sysctl.conf";

/// Every file `sysctl --system` reads on this host: the `*.conf` files
/// of the recognized system directories and `/etc/sysctl.conf`.
pub(super) fn system_files() -> Vec<PathBuf> {
    system_files_in(&DIRS)
}

/// The boot files of `dirs`: one copy per name, from the
/// highest-precedence directory that holds it. procps loads a name
/// once (`PreloadSystem`), so a shadowed copy never reaches the kernel
/// and must not weigh on the boot value. A name masked by a symlink to
/// `/dev/null` — the mechanism `sysctl.d(5)` documents for disabling a
/// lower-precedence file — holds its place like any other copy: the
/// link opens as an empty file, the name is consumed, and the
/// lower-precedence copies of it are the shadowed ones. A `*.conf`
/// basename need not be valid UTF-8 and procps reads it all the same,
/// so the names are compared as their raw bytes: a file whose name is
/// not text is a boot file like any other and must weigh on the
/// analysis.
pub(super) fn system_files_in(dirs: &[(&str, u8)]) -> Vec<PathBuf> {
    // The name a directory holds is claimed by the first directory
    // that lists it: procps compares the directory entry names and
    // never looks at what the entry opens as, so a broken link claims
    // the name all the same and the copies below it are never loaded.
    let mut claimed: BTreeMap<Vec<u8>, Option<PathBuf>> = BTreeMap::new();
    for (dir, _) in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "conf") {
                continue;
            }
            let Some(name) = path.file_name() else {
                continue;
            };
            let key = name.as_bytes().to_vec();
            if claimed.contains_key(&key) {
                continue;
            }
            // Only a copy that opens as a file assigns anything: a
            // broken link and a directory named `*.conf` hold the name
            // and assign nothing.
            let assigns = path.is_file() || masked_by_dev_null(&path);
            claimed.insert(key, assigns.then_some(path));
        }
    }
    let mut files: Vec<PathBuf> = claimed.into_values().flatten().collect();
    files.push(PathBuf::from(LEGACY));
    files
}

/// Whether `path` is a file name masked by a symlink to `/dev/null`:
/// the link opens as an empty file and consumes the name at boot —
/// `sysctl.d(5)` documents the same-name higher-precedence symlink to
/// `/dev/null` as the way to disable a vendor file — while `is_file`,
/// which follows the link, answers false for the character device the
/// link targets. The mask holds the name like any other copy: the
/// lower-precedence files of it never load.
fn masked_by_dev_null(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
        && std::fs::canonicalize(path)
            .is_ok_and(|target| std::fs::canonicalize("/dev/null").is_ok_and(|null| target == null))
}

/// The same-named file in a higher-precedence system directory that
/// shadows the persistence target at boot. `sysctl --system` applies
/// one file name once: after a name is loaded from a directory, the
/// same name in the later directories is ignored, and the directories
/// load by precedence, so a managed file in a lower-precedence
/// directory is silently skipped where a same-named file exists in a
/// higher one, and the parameter reverts after reboot. Only files
/// inside the recognized system directories are shadowed:
/// `/etc/sysctl.conf` loads after all of them and is never skipped.
pub(super) fn shadowing(file: &str) -> Option<String> {
    shadowing_in(file, &DIRS)
}

/// The shadow of `file` among `dirs`: the same name a
/// higher-precedence directory holds.
///
/// The name is held by the entry the directory lists, not by what that
/// entry opens as: procps claims a name by the directory entry
/// (`PreloadSystem`), so a broken link holds the name like any other
/// copy — procps selects it, reports that it cannot open it, and
/// never loads the same name from the lower-precedence directory.
pub(super) fn shadowing_in(file: &str, dirs: &[(&str, u8)]) -> Option<String> {
    shadow_candidates_in(file, dirs)
        .into_iter()
        .find(|candidate| holds_name(candidate))
}

/// The paths that would shadow `file` at boot, highest precedence
/// first: the file's base name in the system directories that outrank
/// its own.
pub(super) fn shadow_candidates(file: &str) -> Vec<String> {
    shadow_candidates_in(file, &DIRS)
}

/// The paths that would shadow `file` at boot among `dirs`, highest
/// precedence first: the file's base name in the system directories
/// that outrank its own.
fn shadow_candidates_in(file: &str, dirs: &[(&str, u8)]) -> Vec<String> {
    let Some((dir, base)) = file.rsplit_once('/') else {
        return Vec::new();
    };
    let dir = format!("{dir}/");
    let Some((_, rank)) = dirs.iter().find(|(known, _)| *known == dir) else {
        return Vec::new();
    };
    dirs.iter()
        .filter(|(_, higher)| higher > rank)
        .map(|(known, _)| format!("{known}{base}"))
        .collect()
}

/// Whether the directory holds an entry with this name: the entry
/// exists, whatever it opens as.
fn holds_name(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The boot files and their contents, read as bytes: a boot file may
/// hold a byte sequence that is not text, and dropping such a file
/// would hide the assignments it makes. A file that cannot be read at
/// all is refused — its assignments are unknown, and an unreadable
/// boot file can restore any value. The paths keep the bytes the host
/// spells them with: the load order compares those bytes, and only
/// the answers that name a file spell it lossily.
pub(super) fn host_files() -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut host = Vec::new();
    for path in system_files() {
        match std::fs::read(&path) {
            Ok(content) => host.push((path, content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot read the boot file {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(host)
}

/// The fingerprint of the effective boot surface: the files that load
/// at boot and what they assign. A change of it — a new file, another
/// assignment — moves the value the host comes up with, so a cached
/// task that judged the host before the change must not be served.
pub(crate) fn boot_fingerprint() -> Option<String> {
    fingerprint_of(&system_files())
}

/// The fingerprint of the boot files `files` names: their raw name
/// bytes and contents, so a file whose name is not text moves the
/// fingerprint like any other.
pub(super) fn fingerprint_of(files: &[PathBuf]) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for path in files {
        let content = match std::fs::read(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        hasher.update(OsStr::new(path).as_bytes());
        hasher.update([0u8]);
        hasher.update(&content);
        hasher.update([0u8]);
    }
    Some(crate::ledger::hex(&hasher.finalize()))
}

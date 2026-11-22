//! The boot surface of sysctl: the files `sysctl --system` reads, the
//! order they load in, and the shadow and override probes over them.
//!
//! procps reads five system directories and the legacy file. It loads
//! one file per name — the copy in the highest-precedence directory
//! that holds it — and the names load together, sorted, with
//! `/etc/sysctl.conf` after all of them.

use std::collections::BTreeMap;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

mod boot_override;
mod fingerprint;
mod keys;
mod shadow;

pub(crate) use boot_override::boot_override;
pub(crate) use fingerprint::boot_fingerprint;
#[cfg(test)]
pub(crate) use fingerprint::fingerprint_of;
#[cfg(test)]
pub(crate) use keys::{key_matches, named_keys};
use shadow::is_self_alias;
pub(crate) use shadow::shadow_candidates_in;

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
const LEGACY: &str = "/etc/sysctl.conf";

/// Every file `sysctl --system` reads on this host: the `*.conf` files
/// of the recognized system directories and `/etc/sysctl.conf`. A
/// directory that cannot be read is an unknown surface, not an empty
/// one — only a missing directory reads as empty — so the error
/// travels with the answer instead of hiding a later-loading
/// assignment from the override preflight and the fingerprint.
pub(super) fn system_files() -> Result<Vec<PathBuf>, String> {
    system_files_in(&DIRS)
}

/// The boot files of `dirs`: one copy per name, from the
/// highest-precedence directory that holds it. procps loads a name
/// once (`PreloadSystem`), so a shadowed copy never reaches the kernel
/// and must not weigh on the boot value. The selection is the name
/// alone: procps records each directory entry's basename before it
/// tries to open it, so a higher entry holds the name whatever it is —
/// a regular file, a mask to `/dev/null`, a broken link, a link to a
/// directory, any other non-regular target — and no lower-precedence
/// copy of the name ever loads. Whether the selected entry contributes
/// its bytes is validated separately, over the selected entries (see
/// `boot_bytes`). A name masked by a symlink to `/dev/null` — the
/// lower-precedence file — holds its place like any other copy: the
/// link opens as an empty file, the name is consumed, and the
/// lower-precedence copies of it are the shadowed ones. A broken
/// symlink holds the name the same way: procps selects it, reports
/// that it cannot open it, and never loads a lower-precedence copy. A
/// `*.conf` basename need not be valid UTF-8 and procps reads it all
/// the same, so the names are compared as their raw bytes: a file
/// whose name is not text is a boot file like any other and must weigh
/// on the analysis.
pub(super) fn system_files_in(dirs: &[(&str, u8)]) -> Result<Vec<PathBuf>, String> {
    let mut by_name: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for (dir, _) in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            // A missing directory holds no names; a directory that
            // exists but cannot be read (permissions, I/O) may hide a
            // later-loading assignment, so it refuses instead of
            // reading as empty.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(scan_error(dir, &error));
            }
        };
        for entry in entries {
            // A per-entry failure refuses like an unreadable directory:
            // dropping it would hide a later-loading assignment from
            // the override probe.
            let entry = entry.map_err(|error| scan_error(dir, &error))?;
            let name = entry.file_name();
            if Path::new(&name).extension().is_none_or(|ext| ext != "conf") {
                continue;
            }
            by_name
                .entry(name.as_bytes().to_vec())
                .or_insert(entry.path());
        }
    }
    let mut files: Vec<PathBuf> = by_name.into_values().collect();
    files.push(PathBuf::from(LEGACY));
    Ok(files)
}

/// The refusal a boot-directory scan failure carries: `read_dir` and
/// per-entry errors refuse identically.
fn scan_error(dir: &str, error: &std::io::Error) -> String {
    format!("cannot read the boot directory {dir}: {error}")
}

/// The bytes the boot reads from a selected entry: its content, or
/// nothing where the entry loads as nothing. A directory, a link to
/// one, a mask to `/dev/null`, and a broken link hold the name the
/// way a regular file does but contribute no assignment, so the
/// lower-precedence copies stay shadowed and no assignment weighs on
/// the boot value. Anything else the loader would block on or misread
/// — a FIFO, a socket, a device that is not the null mask — refuses
/// the surface instead: procps blocks opening a selected FIFO rather
/// than skipping it, so the real loader never reaches later managed
/// files, and approving persistence the apply cannot perform is worse
/// than refusing it. Only a regular file that cannot be read refuses
/// besides those — its assignments are unknown, and an unreadable
/// boot file can restore any value.
pub(super) fn boot_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    // One open, never a pathname twice: the descriptor pins the entry
    // the loader meets, so a swap between the type check and the read
    // can redirect neither the bytes nor the refusal. The open never
    // blocks: a FIFO with no writer would block the loader, so it
    // refuses instead of loading as nothing. Links are followed: a
    // mask to `/dev/null` and an alias load through the entry.
    let fd = match rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) => return open_refusal(path, error),
    };
    let meta = rustix::fs::fstat(&fd).map_err(|error| cannot_read(path, &error.to_string()))?;
    let kind = rustix::fs::FileType::from_raw_mode(meta.st_mode);
    if kind.is_dir() {
        return Ok(None);
    }
    if kind.is_file() {
        // The descriptor is already open, so the entry cannot vanish
        // under the read the way a reopened pathname could.
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut std::fs::File::from(fd), &mut content)
            .map_err(|error| cannot_read(path, &error.to_string()))?;
        return Ok(Some(content));
    }
    // The null mask reads as empty wherever it stands; anything else
    // the loader would block on or misread refuses.
    if is_null_device(&meta) {
        return Ok(None);
    }
    let what = if kind.is_fifo() {
        "a FIFO, which blocks the loader"
    } else if kind.is_socket() {
        "a socket, which holds no assignments"
    } else {
        "a device file, which the loader cannot apply"
    };
    Err(cannot_read(path, &format!("it is {what}")))
}

/// The refusal the open failure carries: what the loader meets when
/// the lookup fails. A missing file — or a link that dangles, loops,
/// or points past an unreachable target — holds the name the way a
/// regular file does but contributes no assignment, while a direct
/// entry the loader cannot open is an unknown surface. A socket never
/// opens, so its kind is read off the entry itself and refuses with
/// its own name rather than a generic open error.
fn open_refusal(path: &Path, error: rustix::io::Errno) -> Result<Option<Vec<u8>>, String> {
    if error == rustix::io::Errno::NOENT
        || std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Ok(None);
    }
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket()) {
        return Err(cannot_read(
            path,
            "it is a socket, which holds no assignments",
        ));
    }
    Err(cannot_read(path, &error.to_string()))
}

/// The refusal a boot read carries, naming the file.
fn cannot_read(path: &Path, why: &str) -> String {
    format!("cannot read the boot file {}: {why}", path.display())
}

/// Whether the open descriptor is the null mask: the character device
/// with `/dev/null`'s own device number, which reads as empty wherever
/// it stands. Any other device refuses — opening it can block or carry
/// side effects the loader never intends. Both numbers come from the
/// same stat family, so no platform's device width leaks into the
/// comparison.
fn is_null_device(meta: &rustix::fs::Stat) -> bool {
    use rustix::fs::FileType;
    if FileType::from_raw_mode(meta.st_mode) != FileType::CharacterDevice {
        return false;
    }
    rustix::fs::stat("/dev/null").is_ok_and(|null| null.st_rdev == meta.st_rdev)
}

/// The same-named file in a higher-precedence system directory that
/// shadows the persistence target at boot. `sysctl --system` applies
/// one file name once: after a name is loaded from a directory, the
/// same name in the later directories is ignored, and the directories
/// load by precedence, so a managed file in a lower-precedence
/// directory is silently skipped where a same-named file exists in a
/// higher one, and the parameter reverts after reboot. The name is
/// what shadows — procps selects it and then reports that it cannot
/// open a broken link, never reaching the lower-precedence copy — so
/// the presence test is `symlink_metadata`, never `exists`, which
/// follows the link and answers false for a broken one. A symlink
/// alias of the target itself is no shadow: the entry resolves to the
/// file itself, so the boot reads the managed bytes through it
/// (`boot_bytes` follows links) and there is nothing to refuse. Only
/// files inside the recognized system directories shadow:
/// `/etc/sysctl.conf` loads after all of them and is never skipped.
///
/// The shadow of `file` among `dirs`, which the tests point at a
/// scratch surface of their own.
pub(super) fn shadowing_in(file: &str, dirs: &[(&str, u8)]) -> Option<String> {
    shadow_candidates_in(file, dirs)
        .into_iter()
        .find(|candidate| {
            std::fs::symlink_metadata(candidate).is_ok() && !is_self_alias(candidate, file)
        })
}

/// The boot files and their contents, read as bytes: a boot file may
/// hold a byte sequence that is not text, and dropping such a file
/// would hide the assignments it makes. A selected entry that loads as
/// nothing — a directory, a link to one, a mask, a broken link — still
/// holds its place: the lower-precedence copies never load, and only
/// the selected entry's (empty) assignments weigh. A regular file that
/// cannot be read at all is refused — its assignments are unknown, and
/// an unreadable boot file can restore any value. The paths keep their
/// raw name bytes: the load order compares them as procps sorts them,
/// and only the answer that names a file spells it.
pub(super) fn host_files() -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut host = Vec::new();
    for path in system_files()? {
        match boot_bytes(&path)? {
            Some(content) => host.push((path, content)),
            None => host.push((path, Vec::new())),
        }
    }
    Ok(host)
}

#[cfg(test)]
#[path = "boot/aliases.rs"]
mod alias_tests;

#[cfg(test)]
#[path = "boot/tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "boot/scratch.rs"]
mod scratch;

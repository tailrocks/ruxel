//! The boot surface of sysctl: the files `sysctl --system` reads, the
//! order they load in, and the shadow and override probes over them.
//!
//! procps reads five system directories and the legacy file. It loads
//! one file per name — the copy in the highest-precedence directory
//! that holds it — and the names load together, sorted, with
//! `/etc/sysctl.conf` after all of them.

use std::collections::BTreeMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{sysctl_key, trimmed_value};

mod fingerprint;
mod keys;

pub(crate) use fingerprint::boot_fingerprint;
#[cfg(test)]
pub(crate) use fingerprint::fingerprint_of;

use keys::{assigned_value, key_matches, named_keys, trim};

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
/// of the recognized system directories and `/etc/sysctl.conf`.
pub(super) fn system_files() -> Vec<PathBuf> {
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
pub(super) fn system_files_in(dirs: &[(&str, u8)]) -> Vec<PathBuf> {
    let mut by_name: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for (dir, _) in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
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
    files
}

/// The bytes the boot reads from a selected entry: its content, or
/// nothing where the entry loads as nothing. A directory, a link to
/// one, and any other non-regular entry — a mask to `/dev/null`, a
/// broken link — hold the name the way a regular file does but
/// contribute no assignment, so the lower-precedence copies stay
/// shadowed and no assignment weighs on the boot value. Reading is
/// never attempted on those: a fifo would block the open the way it
/// blocks procps. Only a regular file that cannot be read refuses —
/// its assignments are unknown, and an unreadable boot file can
/// restore any value.
pub(super) fn boot_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        let kind = meta.file_type();
        if kind.is_dir() || (!kind.is_file() && !kind.is_symlink()) {
            return Ok(None);
        }
        if kind.is_symlink() && !std::fs::metadata(path).is_ok_and(|target| target.is_file()) {
            return Ok(None);
        }
    }
    match std::fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot read the boot file {}: {error}",
            path.display()
        )),
    }
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
/// follows the link and answers false for a broken one. Only files
/// inside the recognized system directories are shadowed:
/// `/etc/sysctl.conf` loads after all of them and is never skipped.
pub(super) fn shadowing(file: &str) -> Option<String> {
    shadowing_in(file, &DIRS)
}

/// The shadow of `file` among `dirs`, which the tests point at a
/// scratch surface of their own.
pub(super) fn shadowing_in(file: &str, dirs: &[(&str, u8)]) -> Option<String> {
    shadow_candidates_in(file, dirs)
        .into_iter()
        .find(|candidate| std::fs::symlink_metadata(candidate).is_ok())
}

/// The paths that would shadow `file` at boot among `dirs`, highest
/// precedence first: the file's base name in the directories that
/// outrank its own.
pub(super) fn shadow_candidates_in(file: &str, dirs: &[(&str, u8)]) -> Vec<String> {
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
    for path in system_files() {
        match boot_bytes(&path)? {
            Some(content) => host.push((path, content)),
            None => host.push((path, Vec::new())),
        }
    }
    Ok(host)
}

/// The later-loading host file that takes the boot value away from
/// the managed entry, if any: its path and the value it leaves behind.
/// The host contents come from outside so the boot surface is
/// provable without a system of its own. A file ruxel manages holds a
/// declared managed value, not an unmanaged override, so `managed` —
/// the file, key and value of every sysctl entry ruxel manages on this
/// host — weighs on the answer. And a wildcard reaches no key the
/// sequence assigns by name: procps keeps the keys a setting names
/// outright out of the expansion of a pattern, so a later
/// `net.ipv4.conf.*.rp_filter` cannot hand the boot value away from a
/// key the managed file assigns by name.
///
/// The boot value is the *last* assignment the sequence makes: the
/// files load in order and the last assignment wins, so an
/// intermediate assignment of another value is overridden by the
/// sequence itself, and only the value the sequence ends with decides
/// what the host comes up with.
pub(super) fn boot_override(
    file: &str,
    name: &str,
    value: &str,
    host: &[(PathBuf, Vec<u8>)],
    managed: &[(String, String, String)],
) -> Option<(String, String)> {
    let key = sysctl_key(name);
    let named = named_assignments(file, name, host);
    let want = trimmed_value(value);
    let mut later: Vec<&(PathBuf, Vec<u8>)> = host
        .iter()
        .filter(|(path, _)| loads_after(file, path))
        .collect();
    later.sort_by(|a, b| load_order(&a.0).cmp(&load_order(&b.0)));
    let mut effective: Option<(&Path, String)> = None;
    for (path, content) in later {
        if let Some(other) = final_assignment(content, &key, &named) {
            effective = Some((path, other));
        }
    }
    let (path, other) = effective?;
    // An assignment of the requested value hands boot the managed
    // value: nothing reverts, whatever loads last.
    if other == want {
        return None;
    }
    if declared(path, &key, &other, managed) {
        return None;
    }
    Some((path.to_string_lossy().into_owned(), other))
}

/// The keys the boot sequence assigns by name, up to and including the
/// managed file: procps keeps a key a setting names outright out of
/// the expansion of a pattern (`write_setting_list` skips a globbed path
/// `settinglist_findpath` finds), so a key named as the sequence loads
/// keeps its value and a later wildcard never reaches it. The managed
/// file's own entry is one such named assignment — the entry the task
/// writes is an assignment by name — so where the file's current
/// content lacks that named assignment (the file is absent, or holds
/// no line for the key yet and `rewrite_file` is about to add one), the
/// entry under management stands for it.
fn named_assignments(file: &str, name: &str, host: &[(PathBuf, Vec<u8>)]) -> Vec<String> {
    let mut named: Vec<String> = host
        .iter()
        .filter(|(path, _)| !loads_after(file, path))
        .flat_map(|(_, content)| named_keys(content))
        .collect();
    let key = sysctl_key(name);
    let named_in_file = host
        .iter()
        .find(|(path, _)| path.as_os_str().as_bytes() == file.as_bytes())
        .is_some_and(|(_, content)| named_keys(content).contains(&key));
    if !named_in_file {
        named.push(key);
    }
    named
}

/// A file ruxel manages: an assignment of the key ruxel declared there
/// is a declared managed value, not an unmanaged boot override. The
/// value must be the declared one — a managed file that drifted to
/// another value carries an unmanaged assignment again.
fn declared(path: &Path, key: &str, value: &str, managed: &[(String, String, String)]) -> bool {
    let bytes = path.as_os_str().as_bytes();
    managed.iter().any(|(file, name, declared_value)| {
        file.as_bytes() == bytes
            && sysctl_key(name) == key
            && trimmed_value(declared_value) == value
    })
}

/// Whether `candidate` loads after `file` at boot: the system files
/// sort together by their base names, and `/etc/sysctl.conf` loads
/// after all of them. One base name loads once, from its
/// highest-precedence directory (see `system_files_in`), so a
/// same-named copy is never a later load.
///
/// The base names are compared as their raw bytes: a `*.conf` basename
/// need not be valid UTF-8, and a lossy spelling would compare
/// differently than procps sorts the names — the replacement
/// character of a byte that is not text sorts before the emoji it
/// displaces, and the load order reverses.
fn loads_after(file: &str, candidate: &Path) -> bool {
    let bytes = candidate.as_os_str().as_bytes();
    if bytes == file.as_bytes() || file == LEGACY {
        return false;
    }
    if bytes == LEGACY.as_bytes() {
        return true;
    }
    let Some(mine) = base_name(file) else {
        return false;
    };
    let Some(theirs) = base_name_bytes(bytes) else {
        return false;
    };
    theirs > mine
}

/// The boot order of a file: the system files load together by their
/// base names, and the legacy file after all of them.
fn load_order(path: &Path) -> (u8, &[u8]) {
    if path == Path::new(LEGACY) {
        return (1, &[]);
    }
    (0, path.file_name().map_or(&[], OsStrExt::as_bytes))
}

/// The base name of a path, as its raw bytes: the name the boot
/// sequence sorts by.
fn base_name(path: &str) -> Option<&[u8]> {
    let (_, name) = path.rsplit_once('/')?;
    Some(name.as_bytes())
}

/// The base name of a path's raw bytes: the name the boot sequence
/// sorts by.
fn base_name_bytes(path: &[u8]) -> Option<&[u8]> {
    let at = path.iter().rposition(|byte| *byte == b'/')?;
    Some(&path[at + 1..])
}

/// The value the file's content leaves the key at: its last matching
/// assignment — procps applies the lines in order and the final one
/// wins. An alias spelling is the same entry, a leading `-` is
/// procps's ignore-failure marker (the assignment happens either
/// way), and a wildcard key assigns every key it expands to — except
/// the keys the sequence assigns by name (`named`): procps keeps those
/// out of the expansion of a pattern, whatever file order the two sit
/// in.
fn final_assignment(content: &[u8], key: &str, named: &[String]) -> Option<String> {
    let mut last: Option<String> = None;
    for line in content.split(|byte| *byte == b'\n') {
        let line = trim(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let assigned = trim(&line[..equals]);
        // procps reads a leading '-' as the ignore-failure marker: the
        // assignment happens either way.
        let assigned = trim(assigned.strip_prefix(b"-").unwrap_or(assigned));
        let assigned = String::from_utf8_lossy(assigned).into_owned();
        if !key_matches(&assigned, key) {
            continue;
        }
        // A wildcard candidate reaches no key the sequence assigns by
        // name: procps excludes the explicitly configured keys from
        // the expansion of a pattern, so the managed key keeps its
        // named value and the pattern assigns none of it.
        if (assigned.contains('*') || assigned.contains('?')) && named.iter().any(|k| k == key) {
            continue;
        }
        last = Some(assigned_value(line));
    }
    last
}

#[cfg(test)]
#[path = "boot/tests.rs"]
mod selection_tests;

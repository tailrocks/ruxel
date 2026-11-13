//! The boot surface of sysctl: the files `sysctl --system` reads, the
//! order they load in, and the shadow and override probes over them.
//!
//! procps reads five system directories and the legacy file. It loads
//! one file per name — the copy in the highest-precedence directory
//! that holds it — and the names load together, sorted, with
//! `/etc/sysctl.conf` after all of them.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use super::{normalized, sysctl_key};

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
/// and must not weigh on the boot value. A `*.conf` basename need not
/// be valid UTF-8 and procps reads it all the same, so the names are
/// compared as their raw bytes: a file whose name is not text is a
/// boot file like any other and must weigh on the analysis.
pub(super) fn system_files_in(dirs: &[(&str, u8)]) -> Vec<PathBuf> {
    let mut by_name: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for (dir, _) in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|ext| ext != "conf") {
                continue;
            }
            let Some(name) = path.file_name() else {
                continue;
            };
            by_name.entry(name.as_bytes().to_vec()).or_insert(path);
        }
    }
    let mut files: Vec<PathBuf> = by_name.into_values().collect();
    files.push(PathBuf::from(LEGACY));
    files
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
    shadow_candidates(file)
        .into_iter()
        .find(|candidate| std::path::Path::new(candidate).exists())
}

/// The paths that would shadow `file` at boot, highest precedence
/// first: the file's base name in the system directories that outrank
/// its own.
pub(super) fn shadow_candidates(file: &str) -> Vec<String> {
    let Some((dir, base)) = file.rsplit_once('/') else {
        return Vec::new();
    };
    let dir = format!("{dir}/");
    let Some((_, rank)) = DIRS.iter().find(|(known, _)| *known == dir) else {
        return Vec::new();
    };
    DIRS.iter()
        .filter(|(_, higher)| higher > rank)
        .map(|(known, _)| format!("{known}{base}"))
        .collect()
}

/// The boot files and their contents, read as bytes: a boot file may
/// hold a byte sequence that is not text, and dropping such a file
/// would hide the assignments it makes. A file that cannot be read at
/// all is refused — its assignments are unknown, and an unreadable
/// boot file can restore any value. The paths are kept as their raw
/// name bytes: the read follows them exactly, while the analysis keys
/// on the lossy spelling only to name the file in an answer.
pub(super) fn host_files() -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut host = Vec::new();
    for path in system_files() {
        match std::fs::read(&path) {
            Ok(content) => host.push((path.to_string_lossy().into_owned(), content)),
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

/// The later-loading host file that assigns `name` to a value of its
/// own, if any: its path and the value. The host contents come from
/// outside so the boot surface is provable without a system of its
/// own. A file ruxel manages holds a declared managed value, not an
/// unmanaged override, so `managed` — the file, key and value of every
/// sysctl entry ruxel manages on this host — weighs on the answer.
pub(super) fn boot_override(
    file: &str,
    name: &str,
    value: &str,
    host: &[(String, Vec<u8>)],
    managed: &[(String, String, String)],
) -> Option<(String, String)> {
    let key = sysctl_key(name);
    host.iter()
        .filter(|(path, _)| loads_after(file, path))
        .find_map(|(path, content)| {
            assigns_other(content, &key, value)
                .into_iter()
                .find(|other| !declared(path, &key, other, managed))
                .map(|other| (path.clone(), other))
        })
}

/// A file ruxel manages: an assignment of the key ruxel declared there
/// is a declared managed value, not an unmanaged boot override. The
/// value must be the declared one — a managed file that drifted to
/// another value carries an unmanaged assignment again.
fn declared(path: &str, key: &str, value: &str, managed: &[(String, String, String)]) -> bool {
    managed.iter().any(|(file, name, declared_value)| {
        file == path && sysctl_key(name) == key && normalized(declared_value) == value
    })
}

/// Whether `candidate` loads after `file` at boot: the system files
/// sort together by their base names, and `/etc/sysctl.conf` loads
/// after all of them. One base name loads once, from its
/// highest-precedence directory (see `system_files_in`), so a
/// same-named copy is never a later load.
fn loads_after(file: &str, candidate: &str) -> bool {
    if candidate == file || file == LEGACY {
        return false;
    }
    if candidate == LEGACY {
        return true;
    }
    match (file.rsplit_once('/'), candidate.rsplit_once('/')) {
        (Some((_, mine)), Some((_, theirs))) => theirs > mine,
        _ => false,
    }
}

/// The values `content` assigns the key to that differ from `value`.
/// The file's effective value is its last matching assignment — procps
/// applies the lines in order and the final one wins — so intermediate
/// assignments of another value are overridden by the file itself and
/// only a final differing value hands the boot value away. An alias
/// spelling is the same entry, a leading `-` is procps's
/// ignore-failure marker (the assignment happens either way), and a
/// wildcard key assigns every key it expands to.
fn assigns_other(content: &[u8], key: &str, value: &str) -> Vec<String> {
    let want = normalized(value);
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
        last = Some(normalized(&String::from_utf8_lossy(trim(
            &line[equals + 1..],
        ))));
    }
    match last {
        Some(other) if other != want => vec![other],
        _ => Vec::new(),
    }
}

/// The ASCII whitespace trimmed off both ends of a line.
fn trim(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |at| at + 1);
    &line[start..end]
}

/// Whether the key a file assigns is the managed key: the two
/// spellings of one parameter are one entry, and a wildcard key
/// (`net.ipv4.conf.*.rp_filter`) assigns every key it expands to.
fn key_matches(candidate: &str, key: &str) -> bool {
    let canonical = sysctl_key(candidate);
    if canonical == key {
        return true;
    }
    if !candidate.contains('*') && !candidate.contains('?') {
        return false;
    }
    glob_matches(&canonical, key)
}

/// A wildcard key against the managed key: `*` stands for any run of
/// characters and `?` for one, both within a single path component —
/// procps expands a pattern one segment at a time (`net.*.rp_filter`
/// assigns no `net.ipv4.conf.all.rp_filter`), so the match stops at
/// canonical-key `/` separators. The match is deliberately wide within
/// a component — a wildcard that could expand to the managed key
/// assigns it — so a later file that restores another value is never
/// missed.
fn glob_matches(pattern: &str, key: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let key: Vec<&str> = key.split('/').collect();
    pattern.len() == key.len()
        && pattern
            .iter()
            .zip(key)
            .all(|(part, name)| segment_matches(part, name))
}

/// One wildcard segment against one path component: `*` any run of
/// characters inside the component, `?` any single one.
fn segment_matches(pattern: &str, key: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let key: Vec<char> = key.chars().collect();
    let (mut at, mut index) = (0, 0);
    let (mut star, mut resume) = (None, 0);
    while index < key.len() {
        if at < pattern.len() && (pattern[at] == '?' || pattern[at] == key[index]) {
            at += 1;
            index += 1;
        } else if at < pattern.len() && pattern[at] == '*' {
            star = Some(at);
            resume = index;
            at += 1;
        } else if let Some(star_at) = star {
            at = star_at + 1;
            resume += 1;
            index = resume;
        } else {
            return false;
        }
    }
    while at < pattern.len() && pattern[at] == '*' {
        at += 1;
    }
    at == pattern.len()
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

//! The boot order of the host's files: which of them load after the
//! managed one, and what they assign.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::super::{normalized, sysctl_key};
use super::LEGACY;

/// The later-loading host file that assigns `name` to a value of its
/// own, if any: its path and the value. The host contents come from
/// outside so the boot surface is provable without a system of its
/// own. A file ruxel manages holds a declared managed value, not an
/// unmanaged override, so `managed` — the file, key and value of every
/// sysctl entry ruxel manages on this host — weighs on the answer. And
/// a wildcard reaches no key the sequence assigns by name: procps
/// keeps the keys a setting names outright out of the expansion of a
/// pattern, so a later `net.ipv4.conf.*.rp_filter` cannot hand the
/// boot value away from a key the managed file assigns by name.
pub(crate) fn boot_override(
    file: &str,
    name: &str,
    value: &str,
    host: &[(PathBuf, Vec<u8>)],
    managed: &[(String, String, String)],
) -> Option<(String, String)> {
    let key = sysctl_key(name);
    let named = named_assignments(file, name, host);
    host.iter()
        .filter(|(path, _)| loads_after(file, path))
        .find_map(|(path, content)| {
            assigns_other(content, &key, value, &named)
                .into_iter()
                .find(|other| !declared(path, &key, other, managed))
                .map(|other| (path.display().to_string(), other))
        })
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
        .find(|(path, _)| same_path(path, file))
        .is_some_and(|(_, content)| named_keys(content).contains(&key));
    if !named_in_file {
        named.push(key);
    }
    named
}

/// The keys `content` assigns by name: every assignment line whose key
/// is no wildcard, in its canonical spelling with procps's
/// ignore-failure marker off.
fn named_keys(content: &[u8]) -> Vec<String> {
    let mut keys = Vec::new();
    for line in content.split(|byte| *byte == b'\n') {
        let line = trim(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let assigned = trim(&line[..equals]);
        let assigned = trim(assigned.strip_prefix(b"-").unwrap_or(assigned));
        let assigned = String::from_utf8_lossy(assigned).into_owned();
        if !assigned.contains('*') && !assigned.contains('?') {
            keys.push(sysctl_key(&assigned));
        }
    }
    keys
}

/// A file ruxel manages: an assignment of the key ruxel declared there
/// is a declared managed value, not an unmanaged boot override. The
/// value must be the declared one — a managed file that drifted to
/// another value carries an unmanaged assignment again.
fn declared(
    path: &Path,
    key: &str,
    value: &str,
    managed: &[(String, String, String)],
) -> bool {
    let bytes = path.as_os_str().as_bytes();
    managed.iter().any(|(file, name, declared_value)| {
        file.as_bytes() == bytes
            && sysctl_key(name) == key
            && normalized(declared_value) == value
    })
}

/// Whether `candidate` loads after `file` at boot: the system files
/// sort together by their base names, and `/etc/sysctl.conf` loads
/// after all of them. One base name loads once, from its
/// highest-precedence directory (see `system_files_in`), so a
/// same-named copy is never a later load.
///
/// The names compare as the bytes the host spells them with: procps
/// sorts the base names with `strcmp` over the raw name, so a name
/// that is not text sorts by the bytes it holds and not by a lossy
/// spelling of them.
fn loads_after(file: &str, candidate: &Path) -> bool {
    let candidate = candidate.as_os_str().as_bytes();
    let managed = file.as_bytes();
    if candidate == managed || file == LEGACY {
        return false;
    }
    if candidate == LEGACY.as_bytes() {
        return true;
    }
    match (base_name(managed), base_name(candidate)) {
        (Some(mine), Some(theirs)) => theirs > mine,
        _ => false,
    }
}

/// The name of the file in a path, as the bytes the host spells it
/// with.
fn base_name(path: &[u8]) -> Option<&[u8]> {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map(|at| &path[at + 1..])
}

/// Whether the path is the one the workflow names: the bytes the host
/// spells the path with, so a name that is not text compares as it is.
fn same_path(path: &Path, file: &str) -> bool {
    path.as_os_str().as_bytes() == file.as_bytes()
}

/// The values `content` assigns the key to that differ from `value`.
/// The file's effective value is its last matching assignment — procps
/// applies the lines in order and the final one wins — so intermediate
/// assignments of another value are overridden by the file itself and
/// only a final differing value hands the boot value away. An alias
/// spelling is the same entry, a leading `-` is procps's
/// ignore-failure marker (the assignment happens either way), and a
/// wildcard key assigns every key it expands to — except the keys the
/// sequence assigns by name (`named`): procps keeps those out of the
/// expansion of a pattern, whatever file order the two sit in.
fn assigns_other(content: &[u8], key: &str, value: &str, named: &[String]) -> Vec<String> {
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
        // A wildcard candidate reaches no key the sequence assigns by
        // name: procps excludes the explicitly configured keys from
        // the expansion of a pattern, so the managed key keeps its
        // named value and the pattern assigns none of it.
        if (assigned.contains('*') || assigned.contains('?')) && named.iter().any(|k| k == key) {
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

/// The ASCII whitespace trimmed off both ends of a byte line.
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

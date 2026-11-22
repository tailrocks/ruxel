//! The boot value the managed entry keeps: the later-loading host file
//! that takes it away, if any.

use super::super::sysctl_key;
use super::keys::{assigned_value, is_pattern, key_matches, named_keys, trim};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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
pub(crate) fn boot_override(
    file: &str,
    name: &str,
    value: &str,
    host: &[(PathBuf, Vec<u8>)],
    managed: &[(String, String, String)],
) -> Option<(String, String)> {
    let key = sysctl_key(name);
    let named = named_assignments(file, name, host);
    // The comparison is on raw bytes: `from_utf8_lossy` collapses
    // distinct values (`\xff` and U+FFFD both spell `�`) that procps
    // passes to the kernel as different bytes. Lossy text below is
    // for the diagnostic only.
    let want = trim(value.as_bytes());
    let mut later: Vec<&(PathBuf, Vec<u8>)> = host
        .iter()
        .filter(|(path, _)| loads_after(file, path))
        .collect();
    later.sort_by(|a, b| load_order(&a.0).cmp(&load_order(&b.0)));
    let mut effective: Option<(&Path, &[u8])> = None;
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
    if declared(path, &key, other, managed) {
        return None;
    }
    Some((
        path.to_string_lossy().into_owned(),
        String::from_utf8_lossy(other).into_owned(),
    ))
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
fn declared(path: &Path, key: &str, value: &[u8], managed: &[(String, String, String)]) -> bool {
    let bytes = path.as_os_str().as_bytes();
    managed.iter().any(|(file, name, declared_value)| {
        file.as_bytes() == bytes
            && sysctl_key(name) == key
            && trim(declared_value.as_bytes()) == value
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
    if bytes == file.as_bytes() || file == super::LEGACY {
        return false;
    }
    if bytes == super::LEGACY.as_bytes() {
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
    if path == Path::new(super::LEGACY) {
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
/// in. The answer is the raw value bytes: lossy text would collapse
/// distinct boot values, so it is for diagnostics only.
fn final_assignment<'a>(content: &'a [u8], key: &str, named: &[String]) -> Option<&'a [u8]> {
    let mut last: Option<&[u8]> = None;
    for line in content.split(|byte| *byte == b'\n') {
        // A trailing carriage return is the line ending, not the
        // value: the syntax-blank trim no longer covers it (managed
        // values forbid it), so the ending is stripped beside the trim
        // the way the file rewrite strips it — a CRLF host file's
        // entry is the same entry, and procps strips it too.
        let line = trim(line.strip_suffix(b"\r").unwrap_or(line));
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
        if is_pattern(&assigned) && named.iter().any(|k| k == key) {
            continue;
        }
        last = Some(assigned_value(line));
    }
    last
}

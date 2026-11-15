//! The keys a boot file assigns: the assignment syntax procps reads,
//! the canonical spelling of one key, and the wildcards a pattern
//! expands to.

use super::super::sysctl_key;

/// The ASCII whitespace trimmed off both ends of a line.
pub(super) fn trim(line: &[u8]) -> &[u8] {
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

/// The keys `content` assigns by name: every assignment line whose key
/// is no wildcard, in its canonical spelling with procps's
/// ignore-failure marker off.
pub(super) fn named_keys(content: &[u8]) -> Vec<String> {
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

/// Whether the key a file assigns is the managed key: the two
/// spellings of one parameter are one entry, and a wildcard key
/// (`net.ipv4.conf.*.rp_filter`) assigns every key it expands to.
pub(super) fn key_matches(candidate: &str, key: &str) -> bool {
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

/// The value an assignment line carries, with the syntax-level
/// whitespace trimmed off both ends: procps strips the blanks around a
/// value, and the blanks inside it are part of the value itself.
pub(super) fn assigned_value(line: &[u8]) -> String {
    let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
        return String::new();
    };
    String::from_utf8_lossy(trim(&line[equals + 1..])).into_owned()
}

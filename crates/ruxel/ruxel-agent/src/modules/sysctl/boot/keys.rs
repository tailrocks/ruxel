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

/// Whether the assignment key is a wildcard pattern: procps expands
/// `*` (any run), `?` (one character) and bracket expressions
/// (`[m]`, `[a-z]`, `[!...]`) through libc `glob` — `GLOB_CHARS` in
/// procps-ng `src/sysctl.c` is `"*?["`, so any `[` starts pattern
/// syntax. Unknown bracket syntax still implies a pattern: the
/// conservative answer excludes the key from `named_keys` and matches
/// it as a pattern, so a possible override is never missed.
pub(crate) fn is_pattern(candidate: &str) -> bool {
    candidate
        .bytes()
        .any(|byte| byte == b'*' || byte == b'?' || byte == b'[')
}

/// The keys `content` assigns by name: every assignment line whose key
/// is no wildcard, in its canonical spelling with procps's
/// ignore-failure marker off. A bracket key is a pattern, not a name.
pub(crate) fn named_keys(content: &[u8]) -> Vec<String> {
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
        if !is_pattern(&assigned) {
            keys.push(sysctl_key(&assigned));
        }
    }
    keys
}

/// Whether the key a file assigns is the managed key: the two
/// spellings of one parameter are one entry, and a wildcard key
/// (`net.ipv4.conf.*.rp_filter`, `vm.[m]ax_map_count`) assigns every
/// key it expands to.
pub(crate) fn key_matches(candidate: &str, key: &str) -> bool {
    let canonical = sysctl_key(candidate);
    if canonical == key {
        return true;
    }
    if !is_pattern(candidate) {
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
/// characters inside the component, `?` any single one, `[...]` the
/// character class libc `glob` reads — the same single-component
/// match the `*` and `?` branches keep, since procps expands a
/// pattern one segment at a time.
fn segment_matches(pattern: &str, key: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let key: Vec<char> = key.chars().collect();
    match_segment(&pattern, &key)
}

/// The segment match as a backtracking matcher over the two
/// character lists.
fn match_segment(pattern: &[char], key: &[char]) -> bool {
    let Some((first, rest)) = pattern.split_first() else {
        return key.is_empty();
    };
    match first {
        '*' => (0..=key.len()).any(|skip| match_segment(rest, &key[skip..])),
        '?' => !key.is_empty() && match_segment(rest, &key[1..]),
        // Unknown bracket syntax implies a pattern: assume the
        // assignment rather than miss an override.
        '[' => match_class(pattern, key).unwrap_or(true),
        literal => key.first() == Some(literal) && match_segment(rest, &key[1..]),
    }
}

/// One `[...]` class against the key's next character: literals,
/// `a-z` ranges and `[!...]`/`[^...]` negation, the way libc `glob`
/// and `fnmatch` (flags 0, as procps calls them) read them. A `]` in
/// first position is a literal member; a reversed range spans
/// nothing. `None`: the class never terminates — unknown syntax, left
/// to the caller's conservative answer.
fn match_class(pattern: &[char], key: &[char]) -> Option<bool> {
    let mut at = 1;
    let negate = matches!(pattern.get(at), Some('!' | '^'));
    if negate {
        at += 1;
    }
    let mut members: Vec<(char, char)> = Vec::new();
    let mut first = true;
    let end = loop {
        let next = pattern.get(at).copied()?;
        at += 1;
        if next == ']' && !first {
            break at - 1;
        }
        first = false;
        let range_hi = match (pattern.get(at), pattern.get(at + 1)) {
            (Some('-'), Some(hi)) if next != '-' && *hi != ']' => Some(*hi),
            _ => None,
        };
        if let Some(hi) = range_hi {
            at += 2;
            members.push((next, hi));
        } else {
            members.push((next, next));
        }
    };
    let Some((head, _)) = key.split_first() else {
        return Some(false);
    };
    let mut hit = members.iter().any(|(lo, hi)| *lo <= *head && *head <= *hi);
    if negate {
        hit = !hit;
    }
    Some(hit && match_segment(&pattern[end + 1..], &key[1..]))
}

/// The value an assignment line carries, as its raw bytes with the
/// syntax-level whitespace trimmed off both ends: procps passes the
/// bytes to the kernel, so a later value of `\xff` and a requested
/// U+FFFD are different values even though `from_utf8_lossy` spells
/// both `�`. Lossy text is for diagnostics only.
pub(super) fn assigned_value(line: &[u8]) -> &[u8] {
    let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
        return &[];
    };
    trim(&line[equals + 1..])
}

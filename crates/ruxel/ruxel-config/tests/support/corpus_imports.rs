//! The relative-import closure of the corpus this harness touches.
//!
//! `touched()` names the examples the harness lists and the fixtures
//! its sources open directly, but an example is not the whole file
//! the evaluator reads: `import "../inventory.pkl"` makes the run open
//! the inventory too, and the inventory's own imports after it. The
//! wiring invariant demands every file in the manifest a declared
//! input, so a helper reached only through an import must be in the
//! manifest — here, transitive, from the source of every file already
//! on the list, bounded by a visited set.
//!
//! The scan runs on the shared source mask (`pkl_mask.rs`), the same
//! one the wiring contract reads with: a form counts only in code and
//! interpolation regions, never in string text or a comment, and the
//! argument is read from the source at the very offset the mask
//! pointed to.

use super::super::{Result, root};
use std::collections::HashSet;
use std::path::Path;

#[path = "pkl_mask.rs"]
mod mask;

use mask::code;

/// Grow `files` with the relative import closure of every corpus file
/// already on it: each file under `tests/workflow` is read once, its
/// relative `import`/`amends` targets resolved against its own
/// directory, and those files walked the same way. A target outside
/// the corpus — a URI scheme, an absolute path, a `..` that leaves
/// `tests/workflow` — names no corpus file and is not followed; a file
/// on the list that cannot be read is a loud failure, never a list
/// quietly short of what the run opens.
pub(super) fn expand(files: &mut Vec<String>) -> Result<()> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut frontier: Vec<String> = files.clone();
    while let Some(file) = frontier.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Some(relative) = file.strip_prefix("tests/workflow/") else {
            continue;
        };
        let path = root().join(relative);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let parent = Path::new(relative)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        for target in imports(&text, relative)? {
            if nonrelative(&target) {
                continue;
            }
            let Some(resolved) = corpus_path(parent, &target) else {
                continue;
            };
            let imported = format!("tests/workflow/{resolved}");
            if !files.contains(&imported) {
                files.push(imported.clone());
            }
            frontier.push(imported);
        }
    }
    Ok(())
}

/// The relative targets one source names: `import` and `amends` in
/// both of Pkl's forms, the statement `import "…"` and the expression
/// `import("…")`, wherever an expression stands and more than once a
/// line. The mask decides where the token is code — `reimport(` is a
/// call of `reimport`, an import inside string text or a comment is
/// text Pkl never loads — and the argument is then read from the
/// source at the offset the mask named.
fn imports(text: &str, name: &str) -> Result<Vec<String>> {
    let mask = code(text);
    let mut targets = Vec::new();
    for form in ["import", "amends"] {
        let mut from = 0;
        while let Some(offset) = mask[from..].find(form) {
            let token = from + offset;
            from = token + form.len();
            if !bounded(&mask, token, form.len()) {
                continue;
            }
            if let Some(target) = argument(text, from, name)? {
                targets.push(target);
            }
        }
    }
    Ok(targets)
}

/// An identifier boundary on both sides of the token: no `reimport(`
/// and no `imports` of a name that merely starts with the word.
fn bounded(mask: &str, token: usize, form: usize) -> bool {
    let bytes = mask.as_bytes();
    let before = token == 0 || {
        let byte = bytes[token - 1];
        !(byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let after = match bytes.get(token + form) {
        Some(byte) => !(byte.is_ascii_alphanumeric() || *byte == b'_'),
        None => true,
    };
    before && after
}

/// The string literal the form carries, read from the source at `at`:
/// the argument sits behind whitespace, a comment or one paren (the
/// expression form), and anything else means this occurrence carries
/// no file — `import` spoken as a bare word names nothing to open.
/// Ordinary, multi-line and raw strings all name a path; an escape or
/// an interpolation inside one would name a path nobody wrote, so
/// they fail loudly instead.
fn argument(text: &str, at: usize, name: &str) -> Result<Option<String>> {
    let bytes = text.as_bytes();
    let mut at = trivia(bytes, at);
    if bytes.get(at) == Some(&b'(') {
        at = trivia(bytes, at + 1);
    }
    match bytes.get(at) {
        Some(b'"') if bytes[at..].starts_with(b"\"\"\"") => {
            let start = at + 3;
            let end = text[start..].find("\"\"\"").ok_or(format!(
                "{name}: a multi-line import argument carries no closing quote"
            ))?;
            Ok(named(&text[start..start + end]))
        }
        Some(b'"') => {
            let start = at + 1;
            let mut end = start;
            while let Some(&byte) = bytes.get(end) {
                match byte {
                    b'"' => return Ok(named(&text[start..end])),
                    b'\\' => {
                        return Err(format!(
                            "{name}: an import argument carries an escape, which a corpus path never needs"
                        )
                        .into());
                    }
                    _ => end += 1,
                }
            }
            Err(format!("{name}: an import argument carries no closing quote").into())
        }
        Some(b'#') => {
            let hashes = bytes[at..].iter().take_while(|&&byte| byte == b'#').count();
            if bytes.get(at + hashes) != Some(&b'"') {
                return Ok(None);
            }
            let start = at + hashes + 1;
            let mut end = start;
            let closing = std::iter::from_fn(|| {
                if end + 1 + hashes > bytes.len() {
                    return None;
                }
                let found = (bytes[end] == b'"'
                    && bytes[end + 1..end + 1 + hashes]
                        .iter()
                        .all(|&byte| byte == b'#'))
                .then_some(end);
                end += 1;
                found
            })
            .next();
            let Some(end) = closing else {
                return Err(
                    format!("{name}: a raw import argument carries no closing quote").into(),
                );
            };
            Ok(named(&text[start..end]))
        }
        // No string follows: `import` spoken as a bare word, or a call
        // whose value is not a string, names nothing to open.
        None | Some(_) => Ok(None),
    }
}

/// The path an argument names: a corpus file needs a plain name, and
/// an empty one names no file at all — `import ""` opens nothing to
/// put on the manifest.
fn named(target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    Some(target.to_owned())
}

/// Whether a target is not a relative corpus import: any URI scheme —
/// `modulepath:` served schema, an `https:` network module an invalid
/// case deliberately imports — or an absolute path. Fed to the
/// resolver, such a target would name a file no check can declare.
fn nonrelative(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return target.starts_with('/');
    };
    scheme.starts_with(|head: char| head.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|head| head.is_ascii_alphanumeric() || matches!(head, '+' | '-' | '.'))
}

/// The corpus path a relative import names, or `None` when it leaves
/// `tests/workflow` — a module outside the corpus is not a corpus
/// file this manifest covers. `.` drops, `..` pops, and a `..` that
/// would pop past the root names nothing.
fn corpus_path(from: &Path, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for piece in from.iter().chain(Path::new(target).iter()) {
        match piece.to_str()? {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// Past whitespace and comments, both of which the source carries
/// between a form and its argument and the mask blanks away.
fn trivia(bytes: &[u8], mut at: usize) -> usize {
    loop {
        if at >= bytes.len() {
            return at;
        }
        if bytes[at].is_ascii_whitespace() {
            at += 1;
        } else if bytes[at..].starts_with(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
        } else {
            return at;
        }
    }
}

//! The Pkl text scanner the corpus contract reads sources with:
//! comment-stripping, import classification, path resolution, string
//! literals. Kept beside `dual_evaluator_corpus.rs` — utilities, not
//! the contract itself.

use super::super::*;

/// The line's code: everything before a `//` comment that sits outside
/// the strings and interpolations a Pkl line carries
/// (`PRRT_kwDOS3Fimc62oWc2`). A `//` inside a string — the `//` of a
/// URL, an interpolation's own text — is string content, not a
/// comment, so an import an interpolation carries survives; one in
/// code ends the line, for nothing after it can be read.
pub(super) fn code(line: &str) -> &str {
    let bytes = line.as_bytes();
    // One depth per open string literal: its still-open `\(...)`
    // interpolations, inside which the rules are code again.
    let mut strings: Vec<usize> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let interpolation = strings.last().is_some_and(|depth| *depth > 0);
        if strings.is_empty() || interpolation {
            if bytes[at..].starts_with(b"//") {
                return &line[..at];
            }
            match bytes[at] {
                b'"' => strings.push(0),
                b'(' if interpolation => {
                    if let Some(depth) = strings.last_mut() {
                        *depth += 1;
                    }
                }
                b')' if interpolation => {
                    if let Some(depth) = strings.last_mut() {
                        *depth -= 1;
                    }
                }
                _ => {}
            }
        } else {
            match bytes[at] {
                // An escape is consumed with its byte; `\(` opens an
                // interpolation instead.
                b'\\' if at + 1 < bytes.len() => {
                    if bytes[at + 1] == b'('
                        && let Some(depth) = strings.last_mut()
                    {
                        *depth += 1;
                    }
                    at += 1;
                }
                b'"' => {
                    strings.pop();
                }
                _ => {}
            }
        }
        at += 1;
    }
    line
}
/// A target no relative corpus import names (`PRRT_kwDOS3Fimc62oWc8`):
/// any URI scheme — `modulepath:` served schema, an `https:` network
/// module an invalid case deliberately imports — or an absolute path;
/// fed to the resolver, such a target would demand a bogus path no
/// check can declare.
pub(super) fn nonrelative(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return target.starts_with('/');
    };
    scheme.starts_with(|head: char| head.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|head| head.is_ascii_alphanumeric() || matches!(head, '+' | '-' | '.'))
}
/// One string literal of an `EXAMPLES` array, read from `at` — the
/// opening quote — and past it. A corpus path needs no escape, and one
/// the text ends before closing names nothing at all.
pub(super) fn literal(body: &[char], at: &mut usize) -> Result<String> {
    *at += 1;
    let mut literal = String::new();
    loop {
        let Some(&head) = body.get(*at) else {
            return Err("an EXAMPLES entry carries no closing quote".into());
        };
        *at += 1;
        match head {
            '"' => break,
            '\\' => {
                return Err(
                    "an EXAMPLES entry carries an escape, which a corpus path never needs".into(),
                );
            }
            _ => literal.push(head),
        }
    }
    Ok(literal)
}
/// The corpus path a relative import names, or `None` when it leaves
/// `tests/workflow` — a module outside the corpus is not a corpus file
/// this contract covers.
pub(super) fn corpus_path(from: &Path, target: &str) -> Option<String> {
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

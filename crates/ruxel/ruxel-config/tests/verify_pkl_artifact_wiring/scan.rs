//! The Pkl text scanner the corpus contract reads sources with:
//! comment-stripping, import classification, path resolution, string
//! literals. Kept beside `dual_evaluator_corpus.rs` — utilities, not
//! the contract itself.
//!
//! The scanner's contract, stated once, is five rules, and the
//! invariant's scope is exactly them:
//!
//! 1. An import is recognized only in code and interpolation regions —
//!    never in literal string text (`"..."`, `"""..."""`, or a
//!    raw `#"..."#` string) or a comment, line `//` or block
//!    `/* ... */` — in either of Pkl's forms, the statement `import "…"`
//!    / `amends "…"` and the expression `import("…")` / `amends("…")`,
//!    wherever an expression stands, more than once a line.
//! 2. `modulepath:` and every URI scheme, and every absolute path, are
//!    excluded: nonrelative targets are never followed.
//! 3. Only relative corpus paths under `tests/workflow` are followed,
//!    transitively, the walk bounded by a visited set.
//! 4. Every harness source the contract discovers — each `[[test]]`
//!    target gated behind the `dual-evaluator` feature, and every
//!    module file under its sibling directory, at any depth — must
//!    itself be a declared input of the `dual-evaluator` check, and
//!    the mise `dual-evaluator` task must run every discovered
//!    harness: a corpus the mandatory task never executes is a corpus
//!    nobody checks.
//! 5. A file outside these rules is a scope question, not a defect:
//!    the harnesses read only what these rules name.

use super::super::*;

/// The code of a Pkl source as a mask over it, byte for byte: the
/// bytes its evaluator parses are kept, string-literal text and
/// comment text become blanks (`PRRT_kwDOS3Fimc62oWc2`,
/// `PRRT_kwDOS3Fimc62pCSD`). A `//` inside a string — the `//` of a
/// URL — is content, not a comment; an `import(` inside a string is
/// doc text Pkl never loads, `"..."` or a `"""` multi-line string
/// alike; the same call inside a string's `\(...)` interpolation is
/// code and stays. A quote that opens a string from code stays too,
/// for the statement form ends in one. Offsets are preserved — every
/// byte of the source answers one byte of the mask — so a form found
/// in the mask names its target at the same offset of the source.
/// One open string literal: `"..."`, or `"""..."""` multi-line.
#[derive(Clone, Copy)]
enum Delimited {
    Single,
    Multi,
}

/// The enclosing regions, innermost last: a string literal, or a
/// `\(...)` interpolation of the string beneath it — code again,
/// with its own nesting of parens and strings.
enum Frame {
    Str(Delimited),
    Interp(usize),
}

/// One byte of a string literal's region: its text is blanked, an
/// escape is consumed with its byte, `\(` opens an interpolation, and
/// a closing quote ends the literal. The return is the bytes consumed
/// beyond the one the caller advances.
fn string_byte(frames: &mut Vec<Frame>, bytes: &[u8], at: usize, mask: &mut Vec<u8>) -> usize {
    match bytes[at] {
        b'\\' if at + 1 < bytes.len() => {
            if bytes[at + 1] == b'(' {
                frames.push(Frame::Interp(1));
                mask.push(b'(');
            } else {
                mask.push(b' ');
            }
            1
        }
        b'\n' => {
            mask.push(b'\n');
            0
        }
        b'"' if bytes[at..].starts_with(b"\"\"\"")
            && matches!(frames.last(), Some(Frame::Str(Delimited::Multi))) =>
        {
            frames.pop();
            mask.extend_from_slice(b"\"\"\"");
            2
        }
        b'"' if matches!(frames.last(), Some(Frame::Str(Delimited::Single))) => {
            frames.pop();
            mask.push(b'"');
            0
        }
        _ => {
            mask.push(b' ');
            0
        }
    }
}

/// A raw string's extent (`PRRT_kwDOS3Fimc62pCSD`'s class, the raw
/// form): one or more `#` then a quote opens it, and only a quote plus
/// as many `#` closes it. `None` when the bytes at `at` open no raw
/// string; the whole literal is text Pkl never scans for imports.
fn raw_string(bytes: &[u8], at: usize) -> Option<usize> {
    let hashes = bytes[at..].iter().take_while(|&&byte| byte == b'#').count();
    if bytes.get(at + hashes) != Some(&b'"') {
        return None;
    }
    let opener = &bytes[at..at + hashes];
    let mut end = at + hashes + 1;
    while end < bytes.len() {
        if bytes[end] == b'"' && bytes[end + 1..].starts_with(opener) {
            return Some(end + 1 + hashes);
        }
        end += 1;
    }
    Some(bytes.len())
}

pub(super) fn code(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut mask = Vec::with_capacity(bytes.len());
    let mut frames: Vec<Frame> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let in_string = matches!(frames.last(), Some(Frame::Str(_)));
        if in_string {
            at += string_byte(&mut frames, bytes, at, &mut mask);
        } else {
            if bytes[at..].starts_with(b"//") {
                // A comment runs to the line's end and reads nothing.
                while at < bytes.len() && bytes[at] != b'\n' {
                    mask.push(b' ');
                    at += 1;
                }
                continue;
            }
            if let Some(end) = raw_string(bytes, at) {
                for byte in &bytes[at..end] {
                    mask.push(if *byte == b'\n' { b'\n' } else { b' ' });
                }
                at = end;
                continue;
            }
            if bytes[at..].starts_with(b"/*") {
                // A block comment runs to its `*/` and reads nothing
                // (`PRRT_kwDOS3Fimc62plX3`).
                let close = bytes[at + 2..]
                    .windows(2)
                    .position(|window| window == b"*/")
                    .map(|skip| at + 2 + skip + 2);
                let end = close.unwrap_or(bytes.len());
                for byte in &bytes[at..end] {
                    mask.push(if *byte == b'\n' { b'\n' } else { b' ' });
                }
                at = end;
                continue;
            }
            if bytes[at..].starts_with(b"\"\"\"") {
                frames.push(Frame::Str(Delimited::Multi));
                mask.extend_from_slice(b"\"\"\"");
                at += 2;
            } else {
                match bytes[at] {
                    b'\n' => mask.push(b'\n'),
                    b'"' => {
                        frames.push(Frame::Str(Delimited::Single));
                        mask.push(b'"');
                    }
                    b'(' => {
                        if let Some(Frame::Interp(depth)) = frames.last_mut() {
                            *depth += 1;
                        }
                        mask.push(b'(');
                    }
                    b')' => {
                        mask.push(b')');
                        if let Some(Frame::Interp(depth)) = frames.last_mut() {
                            *depth -= 1;
                        }
                        if matches!(frames.last(), Some(Frame::Interp(0))) {
                            frames.pop();
                        }
                    }
                    byte => mask.push(byte),
                }
            }
        }
        at += 1;
    }
    String::from_utf8_lossy(&mask).into_owned()
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

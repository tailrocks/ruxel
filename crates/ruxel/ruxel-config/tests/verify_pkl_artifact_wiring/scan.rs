//! The Pkl text scanner the corpus contract reads sources with. Kept
//! beside `dual_evaluator_corpus.rs` — utilities, not the contract
//! itself.
//!
//! The scanner's contract, stated once, is five rules, and the
//! invariant's scope is exactly them:
//!
//! 1. An import is recognized only in code and interpolation regions —
//!    never in literal string text (`"..."`, `"""..."""`, or a raw
//!    `#"..."#` string) or a comment, line `//` or block `/* ... */` —
//!    in either of Pkl's forms, the statement `import "…"` /
//!    `amends "…"` and the expression `import("…")` / `amends("…")`
//!    (a raw string may carry the argument), wherever an expression
//!    stands, more than once a line, and always at an identifier
//!    boundary: `reimport(` is a call of `reimport`, not an import.
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

/// One open region of a source, innermost last.
enum Region {
    /// A `"…"` string, or a `"""…"""` multi-line one.
    Str { multi: bool },
    /// A `#"…"#` raw string; `hashes` is its opening `#` count.
    Raw { hashes: usize },
    /// A `\(...)` interpolation of the string beneath it — code again,
    /// with its own nesting of parens and strings.
    Interp { depth: usize },
}

/// The code of a Pkl or Rust source as a mask over it: one mask byte
/// per source byte, always — the byte the evaluator parses stay, the
/// rest become blanks, newlines stay newlines. A `//` inside a string
/// is content, not a comment; an `import(` inside a string or a
/// comment is text Pkl never loads; the same call inside a string's
/// `\(...)` interpolation is code and stays; a raw string ends at `"`
/// plus its `#`s, never at a bare interior quote.
/// A comment's extent from `at`: a `//` runs to its line's end, a
/// `/*` to its `*/` — comment text parses nothing and keeps nothing.
/// `None` when no comment opens at `at`.
fn comment_end(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes[at..].starts_with(b"//") {
        return Some(
            bytes[at..]
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(bytes.len(), |newline| at + newline),
        );
    }
    if bytes[at..].starts_with(b"/*") {
        let mut end = at + 2;
        while end < bytes.len() && !bytes[end..].starts_with(b"*/") {
            end += 1;
        }
        return Some((end + 2).min(bytes.len()));
    }
    None
}

/// One step through a string literal's region: an escape is consumed
/// with its byte, `\(` opens an interpolation, a `"""` closes a
/// multi-line string and a bare `"` an ordinary one. The return is the
/// bytes consumed beyond the one the caller advances.
fn string_step(regions: &mut Vec<Region>, bytes: &[u8], at: usize) -> usize {
    let multi = matches!(regions.last(), Some(Region::Str { multi: true }));
    if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'(') {
        regions.push(Region::Interp { depth: 1 });
        return 1;
    }
    if bytes[at] == b'\\' && at + 1 < bytes.len() {
        return 1;
    }
    let closes = match (multi, bytes[at..].starts_with(b"\"\"\"")) {
        (true, true) => 2,
        (false, false) if bytes[at] == b'"' => 0,
        _ => return 0,
    };
    if let Some(Region::Str { .. }) = regions.last() {
        regions.pop();
    }
    closes
}

pub(super) fn code(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut kept = vec![false; bytes.len()];
    let mut regions: Vec<Region> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match regions.last_mut() {
            // Code, or code inside an interpolation: the same rules.
            None | Some(Region::Interp { .. }) => {
                if let Some(end) = comment_end(bytes, at) {
                    at = end;
                } else if let Some((hashes, end)) = raw_delimiter(bytes, at) {
                    at = end;
                    regions.push(Region::Raw { hashes });
                } else if bytes[at..].starts_with(b"\"\"\"") {
                    for flag in &mut kept[at..at + 3] {
                        *flag = true;
                    }
                    at += 3;
                    regions.push(Region::Str { multi: true });
                } else if bytes[at] == b'"' {
                    kept[at] = true;
                    at += 1;
                    regions.push(Region::Str { multi: false });
                } else if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'(') {
                    at += 2;
                    regions.push(Region::Interp { depth: 1 });
                } else if matches!(regions.last(), Some(Region::Interp { .. }))
                    && matches!(bytes[at], b'(' | b')')
                {
                    // An interpolation's parens nest; its closing paren
                    // hands control back to the string.
                    if let Some(Region::Interp { depth }) = regions.last_mut() {
                        match (bytes[at], *depth) {
                            (b'(', _) => *depth += 1,
                            (b')', 1) => {
                                regions.pop();
                            }
                            (b')', _) => *depth -= 1,
                            _ => {}
                        }
                    }
                    kept[at] = true;
                    at += 1;
                } else {
                    kept[at] = true;
                    at += 1;
                }
            }
            Some(Region::Raw { hashes }) => {
                // Only `"` followed by as many `#` closes a raw string.
                let hashes = *hashes;
                if bytes[at] == b'"'
                    && bytes[at + 1..]
                        .iter()
                        .take(hashes)
                        .all(|&byte| byte == b'#')
                {
                    at += 1 + hashes;
                    regions.pop();
                } else {
                    at += 1;
                }
            }
            Some(Region::Str { .. }) => {
                at += 1 + string_step(&mut regions, bytes, at);
            }
        }
    }
    kept.iter()
        .zip(bytes)
        .map(|(&flag, &byte)| if flag || byte == b'\n' { byte } else { b' ' } as char)
        .collect()
}

/// A raw string's opening delimiter at `at`: its `#` count and the
/// offset just past its quote. `None` when fewer than one `#` or no
/// quote follows.
fn raw_delimiter(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let hashes = bytes[at..].iter().take_while(|&&byte| byte == b'#').count();
    if hashes == 0 || bytes.get(at + hashes) != Some(&b'"') {
        return None;
    }
    Some((hashes, at + hashes + 1))
}

/// The offset where a source declares its corpus: the first
/// occurrence of the declaration outside comments and string text —
/// the mask blanks those, so a documented or retired example cannot
/// hijack the parse.
pub(super) fn declared_at(text: &str, declaration: &str) -> Option<usize> {
    code(text).find(declaration)
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

/// The declaration lookup ignores comments and string text: a
/// documented or retired `EXAMPLES` cannot hijack the parse.
#[test]
fn the_declaration_lookup_ignores_comments_and_strings() {
    let text = "// const EXAMPLES: &[&str] = &[\"bogus.pkl\"];\n".to_owned()
        + "/* const EXAMPLES: &[&str] = &[\"also-bogus.pkl\"]; */\n"
        + "const EXAMPLES: &[&str] = &[";
    let at =
        declared_at(&text, "const EXAMPLES: &[&str] = &[").expect("the live declaration is found");
    assert_eq!(
        &text[at..at + "const EXAMPLES: &[&str] = &[".len()],
        "const EXAMPLES: &[&str] = &["
    );
    assert!(declared_at(&text, "also-bogus").is_none());
}

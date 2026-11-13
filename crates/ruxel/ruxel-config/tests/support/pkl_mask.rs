//! The Pkl source mask both lanes scan with: one byte per source
//! byte, the code kept and literal text and comments blanked, so an
//! offset in the mask names the very byte in the source.
//!
//! The mask's rules, stated once here and relied on by every reader:
//!
//! 1. An import is recognized only in code and interpolation regions —
//!    never in literal string text (`"..."`, `"""..."""`, or a raw
//!    `#"..."#` string) or a comment, line `//` or block `/* ... */`.
//! 2. A `//` inside a string is content, not a comment; the same `//`
//!    in code or an interpolation starts one.
//! 3. An interpolation `\(...)` is code again, with its own nesting of
//!    parens and strings, so an import one carries stays visible.
//! 4. A raw string ends at a `"` plus as many `#`s as it opened with,
//!    never at a bare interior quote.
//! 5. The output is byte-aligned with the input: one byte out per byte
//!    in, newlines kept, every other blanked byte a space.

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
    // One byte out per byte in, built through the source's own
    // characters: a character the mask kept is the source's own, one
    // it blanked becomes as many spaces as it was bytes long, so the
    // mask stays byte-aligned with the source — an offset found here
    // names the very byte there, even past non-ASCII text, which a
    // `byte as char` reinterpretation would not. A newline stays a
    // newline whatever blanked it, so the mask still cuts the source's
    // lines.
    let mut masked = String::with_capacity(text.len());
    for (offset, character) in text.char_indices() {
        let width = character.len_utf8();
        if character == '\n' || kept[offset..offset + width].iter().all(|&flag| flag) {
            masked.push(character);
        } else {
            for _ in 0..width {
                masked.push(' ');
            }
        }
    }
    masked
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

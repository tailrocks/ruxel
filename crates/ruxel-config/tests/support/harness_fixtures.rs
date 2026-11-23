//! The fixtures a harness source opens directly: the `root().join("…")`
//! literals a source really spells.
//!
//! A harness source opens the fixtures it copies or reads by a literal
//! `root().join("…")` — the chain and join-separator sweeps copy the
//! shared inventory that way — and the manifest the wiring contract
//! reads must name every one of them.
//!
//! The join is located through the shared source mask
//! (`pkl_mask.rs`), so a commented-out or documented join — text the
//! mask blanks — names no file, and the literal itself is read from
//! the source at the offset the mask named, because the mask blanks a
//! string's text. A raw substring scan would read `retired.pkl` out of
//! a comment and demand a file the harness never opens.
//!
//! A join whose argument is not a string literal names no fixture the
//! manifest can spell: the harness's own `root().join(example)` opens
//! the examples, and the corpus names those already.

use super::super::Result;
use super::mask::code;

/// The fixtures a harness source opens by a literal `root().join("…")`,
/// in the order the source spells them.
pub(super) fn fixtures(source: &str) -> Result<Vec<String>> {
    let mask = code(source);
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut from = 0;
    while let Some(offset) = mask[from..].find("root().join(") {
        let token = from + offset;
        from = token + "root().join(".len();
        let mut at = from;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) != Some(&b'"') {
            // A join whose argument is not a literal opens no file the
            // manifest can name: the harness's own
            // `root().join(example)` opens the examples, and the
            // corpus names those already.
            continue;
        }
        let start = at + 1;
        let mut end = start;
        loop {
            match bytes.get(end) {
                Some(b'"') => break,
                Some(b'\\') => end += 2,
                Some(_) => end += 1,
                None => return Err(
                    "a harness root().join carries no closing quote; name the fixture in touched()"
                        .into(),
                ),
            }
        }
        literals.push(source[start..end].to_owned());
        from = end;
    }
    Ok(literals)
}

/// A commented-out or documented join opens no fixture: the mask blanks
/// the comment, so the scan never sees it. A raw substring scan would
/// read `retired.pkl` out of the comment and demand a file the harness
/// never opens.
#[test]
fn a_commented_join_names_no_fixture() {
    let source = concat!(
        "// root().join(\"retired.pkl\")\n",
        "/* root().join(\"also-retired.pkl\") */\n",
        "let path = root().join(\"live.pkl\");\n",
    );
    assert_eq!(
        fixtures(source).expect("the live join carries its literal"),
        ["live.pkl".to_owned()]
    );
}

/// A join whose argument is not a literal names no fixture: the
/// harness's own `root().join(example)` opens the examples the corpus
/// already names.
#[test]
fn a_join_without_a_literal_names_no_fixture() {
    let source = "let path = root().join(example);\n";
    assert_eq!(
        fixtures(source).expect("the join carries no literal"),
        Vec::<String>::new()
    );
}

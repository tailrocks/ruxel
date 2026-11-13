//! The Pkl text scanner the corpus contract reads sources with. Kept
//! beside `dual_evaluator_corpus.rs` — utilities, not the contract
//! itself.
//!
//! The mask that draws code apart from literal text and comments is
//! not this file's own anymore: it lives in `../support/pkl_mask.rs`,
//! shared with the gated harnesses that compute the runtime corpus
//! manifest from it, so both lanes read Pkl source through one
//! scanner with one set of rules. What stays here is what only this
//! lane needs: the declaration lookup that ignores comments and
//! string text, and the array literal reader.

use super::super::*;

/// The source mask, shared with the gated harnesses: one scanner for
/// both lanes, so a rule that holds for the contract's own parse
/// holds for the manifest the harnesses verify.
#[path = "../support/pkl_mask.rs"]
mod mask;

use mask::code;

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

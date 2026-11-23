//! The fnmatch matcher the corpus contract classifies inputs with.
//! Kept beside `dual_evaluator_corpus.rs` — a utility, not the contract
//! itself — so the contract module stays within the repository's file
//! size policy.

/// fnmatch semantics, the classification a check's `inputs` carry: `*`
/// matches any run of bytes, a slash included, because a pattern names
/// a whole path; `?` one byte; a bracket expression one byte of its
/// set, `!` or `^` at its head negating. Every other byte matches
/// itself, so a plain-path input matches that path and nothing else —
/// the form every committed input takes.
pub(super) fn matches(pattern: &str, path: &str) -> bool {
    fn one(pattern: &[u8], path: &[u8]) -> bool {
        let Some((&head, rest)) = pattern.split_first() else {
            return path.is_empty();
        };
        match head {
            b'*' => (0..=path.len()).any(|skip| one(rest, &path[skip..])),
            b'?' => !path.is_empty() && one(rest, &path[1..]),
            b'[' => {
                // The set runs to the first `]`, which a leading one never
                // closes; an expression that never closes matches nothing.
                let opened = usize::from(rest.first() == Some(&b']'));
                let Some(close) = rest[opened..]
                    .iter()
                    .position(|byte| *byte == b']')
                    .map(|at| at + opened)
                else {
                    return false;
                };
                let set = &rest[..close];
                let tail = &rest[close + 1..];
                let Some(&byte) = path.first() else {
                    return false;
                };
                let (negated, set) = match set.first() {
                    Some(b'!' | b'^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut at = 0;
                while at < set.len() {
                    if at + 2 < set.len() && set[at + 1] == b'-' {
                        hit |= set[at] <= byte && byte <= set[at + 2];
                        at += 3;
                    } else {
                        hit |= set[at] == byte;
                        at += 1;
                    }
                }
                hit != negated && one(tail, &path[1..])
            }
            byte => path.first() == Some(&byte) && one(rest, &path[1..]),
        }
    }
    one(pattern.as_bytes(), path.as_bytes())
}

/// The inputs of one generated check that match a path, in the fnmatch
/// sense the classification applies.
pub(super) fn matching_inputs<'a>(inputs: &'a [&'a str], path: &str) -> Vec<&'a str> {
    inputs
        .iter()
        .copied()
        .filter(|input| matches(input, path))
        .collect()
}

/// The matcher is fnmatch, not a formality: a matcher that matched
/// nothing — or everything — would vacate the contracts above while
/// still passing.
#[test]
fn the_input_matcher_is_fnmatch() {
    assert!(matches(
        "tests/workflow/valid/user-missing-account.pkl",
        "tests/workflow/valid/user-missing-account.pkl"
    ));
    assert!(!matches(
        "tests/workflow/valid/user-missing-account.pkl",
        "tests/workflow/valid/tags.pkl"
    ));
    assert!(matches(
        "tests/workflow/valid/*.pkl",
        "tests/workflow/valid/user-missing-account.pkl"
    ));
    assert!(matches("tests/*", "tests/workflow/valid/tags.pkl")); // `*` crosses a slash
    assert!(matches("a?c", "abc"));
    assert!(!matches("a?c", "ac"));
    assert!(matches("[a-c]b", "bb"));
    assert!(matches("[!x]b", "yb"));
    assert!(!matches("[!x]b", "xb"));
    assert!(!matches("[ab", "a"));
    assert!(!matches("*.pkl", "corpus.pkl.txt"));
}

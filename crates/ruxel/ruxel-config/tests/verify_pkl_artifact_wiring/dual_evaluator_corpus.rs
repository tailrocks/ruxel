//! The dual-evaluator corpus as a contract with `.velnor/config.toml`:
//! every file the target reads must be an input of the `dual-evaluator`
//! check, or a PR that changes only that file classifies the mandatory
//! `Check / dual-evaluator` job unaffected. The corpus lives in a
//! feature-gated target the pkl-less lanes skip, so an omission there
//! cannot catch itself: this target is an ordinary one of the same lane,
//! and it reads the corpus source rather than running it.

use super::*;

/// fnmatch semantics, the classification a check's `inputs` carry: `*`
/// matches any run of bytes, a slash included, because a pattern names a
/// whole path and never a fragment of one; `?` matches one byte; a
/// bracket expression matches one byte of its set, with `!` or `^` at
/// its head negating. Every other byte matches itself, so an input
/// written as a plain path matches that path and nothing else — the form
/// every committed input takes.
fn matches(pattern: &str, path: &str) -> bool {
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
fn matching_inputs<'a>(inputs: &'a [&'a str], path: &str) -> Vec<&'a str> {
    inputs
        .iter()
        .copied()
        .filter(|input| matches(input, path))
        .collect()
}

/// The dual-evaluator corpus: the `EXAMPLES` array
/// `crates/ruxel/ruxel-config/tests/dual_evaluator.rs` declares, resolved
/// against `tests/workflow` the way that suite's own `root()` resolves
/// them. The array is parsed structurally, element by element: a string
/// literal is an entry, whitespace, commas and `//` comments separate
/// them, and anything else — a named constant, a concatenation, a macro —
/// is a loud failure, never an element the parse silently skips while
/// the invariants above pass over the path it names. The terminator is
/// part of that structure (`PRRT_kwDOS3Fimc62jyW4`): the array ends at
/// the `];` the scan reaches at an element boundary, never at one a
/// comment or a string hides, because the comment branch consumes the
/// comment's text — a `];` inside one — long before the scan looks for
/// a delimiter, so a comment cannot end the array early and leave the
/// entries after it unchecked. It is the only explicit corpus list the
/// repo keeps — the admission suites scan directories and name their
/// files per test — so a corpus that stops being an array of literals
/// there is a corpus this parse cannot see, and it says so rather than
/// passing over an unread contract.
fn examples() -> Result<Vec<String>> {
    let path = repo().join("crates/ruxel/ruxel-config/tests/dual_evaluator.rs");
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let declaration = "const EXAMPLES: &[&str] = &[";
    let start = text
        .find(declaration)
        .ok_or("dual_evaluator.rs declares no EXAMPLES corpus")?;
    let body: Vec<char> = text[start + declaration.len()..].chars().collect();
    let mut examples = Vec::new();
    let mut at = 0;
    let mut terminated = false;
    while at < body.len() {
        match body[at] {
            head if head.is_whitespace() => at += 1,
            '/' if body.get(at + 1) == Some(&'/') => {
                while at < body.len() && body[at] != '\n' {
                    at += 1;
                }
            }
            ',' => at += 1,
            ']' => {
                // Only a `]` the scan reaches between elements can close
                // the array: inside the loop the scan always sits at an
                // element boundary, and a `];` a comment carries never
                // gets here. The semicolon may carry whitespace before
                // it; a `]` that closes nothing is a loud failure, never
                // a delimiter the parse accepts by accident.
                let semicolon = body[at + 1..]
                    .iter()
                    .position(|head| !head.is_whitespace())
                    .map(|skip| at + 1 + skip);
                match semicolon {
                    Some(at_semicolon) if body[at_semicolon] == ';' => {
                        terminated = true;
                        break;
                    }
                    _ => {
                        return Err("the EXAMPLES corpus carries a `]` that closes no array".into());
                    }
                }
            }
            '"' => {
                at += 1;
                let mut example = String::new();
                loop {
                    let Some(&head) = body.get(at) else {
                        return Err("an EXAMPLES entry carries no closing quote".into());
                    };
                    at += 1;
                    match head {
                        '"' => break,
                        // A corpus path needs no escape, and silently
                        // unescaping one would name a path nobody wrote.
                        '\\' => return Err(
                            "an EXAMPLES entry carries an escape, which a corpus path never needs"
                                .into(),
                        ),
                        _ => example.push(head),
                    }
                }
                examples.push(example);
            }
            _ => {
                let mut element = String::new();
                while at < body.len() && body[at] != ',' && body[at] != '\n' {
                    element.push(body[at]);
                    at += 1;
                }
                return Err(format!(
                    "the EXAMPLES corpus carries an element that is not a string literal: {}",
                    element.trim()
                )
                .into());
            }
        }
    }
    if !terminated {
        return Err("the EXAMPLES corpus carries no terminator".into());
    }
    if examples.is_empty() {
        return Err("the EXAMPLES corpus parsed empty".into());
    }
    Ok(examples)
}

/// The corpus path a relative import names, or `None` when it leaves
/// `tests/workflow` — a module outside the corpus is not a corpus file
/// this contract covers.
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

/// The corpus files one file reads beyond itself: the targets of its
/// `import` and `amends` clauses that name a relative path, resolved
/// against the file's own directory, in either of Pkl's two forms — the
/// statement `import "path"` and the expression `import("path")`, an
/// `amends` amending the same way. `PRRT_kwDOS3Fimc62jyWw`: the
/// evaluator honours both forms
/// (`crates/ruxel/ruxel-config/src/eval/confined/tests.rs` reaches a
/// module through `import "..."` and a resource through an expression),
/// so a file the corpus reaches through the expression form alone is a
/// file this scan must see, or it alone skips the dual-evaluator check.
/// `modulepath:` targets are the served schema, whose inputs are another
/// contract.
fn imports(file: &str) -> Result<Vec<String>> {
    let path = repo().join("tests/workflow").join(file);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let parent = Path::new(file)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut imports = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = ["import \"", "amends \"", "import(", "amends("]
            .iter()
            .find_map(|form| line.strip_prefix(form))
        else {
            continue;
        };
        // The statement forms have spent their opening quote in the
        // prefix; the expression forms still carry it, possibly behind
        // whitespace, so both reach the path the same way.
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('"').unwrap_or(rest);
        let (target, _) = rest
            .split_once('"')
            .ok_or(format!("{file}: an import carries no closing quote"))?;
        if target.starts_with("modulepath:") {
            continue;
        }
        if let Some(path) = corpus_path(parent, target) {
            imports.push(format!("tests/workflow/{path}"));
        }
    }
    imports.sort();
    imports.dedup();
    Ok(imports)
}

/// `PRRT_kwDOS3Fimc62ioGG`: every workflow the dual-evaluator corpus
/// names must be an input of the `dual-evaluator` check. The corpus and
/// the input list are maintained by hand and can drift: PR #90 added
/// `valid/user-missing-account.pkl` to the corpus without declaring it,
/// and a PR that changed only that file classified the mandatory
/// `Check / dual-evaluator` job unaffected. The suite that would notice
/// is the one being skipped, so the contract lives here, in the ordinary
/// lane that always runs.
#[test]
fn every_dual_evaluator_example_is_a_declared_input() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    for example in examples()? {
        let path = format!("tests/workflow/{example}");
        if !repo().join(&path).is_file() {
            return Err(format!("EXAMPLES names {path}, which is not a committed file").into());
        }
        if matching_inputs(&inputs, &path).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// The corpus the dual-evaluator reads beyond the examples it names,
/// derived from what the target actually opens rather than from a list
/// kept beside it: the inventory its chain and join-separator suites copy
/// through their `root()`, and every corpus file a listed example imports
/// by a relative path — the only way a case reaches a file beyond itself.
/// `PRRT_kwDOS3Fimc62jyWq`: the walk is transitive. An example that
/// imports an aggregator — `recipes.pkl` — makes the evaluator open the
/// aggregator's own imports too (`recipes/api-proxy.pkl`,
/// `recipes/kubeadm-join.pkl`), so a corpus file reached only through an
/// aggregator is still a file the harness opens, and the visited set
/// bounds the walk: every file is parsed once, and a cycle a future
/// import graph grows cannot loop it.
/// `PRRT_kwDOS3Fimc62jyXrx`: nothing the suite never opens is required
/// here. No example imports a recipe today, so `recipes.pkl` and its
/// leaves are not demanded, though the check may still declare them for
/// its own reasons; the day an example imports one, it becomes required,
/// because that is the day the suite reads it.
#[test]
fn every_corpus_file_the_dual_evaluator_reads_is_declared() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    let mut read = vec!["tests/workflow/inventory.pkl".to_owned()];
    let mut frontier: Vec<String> = examples()?;
    let mut visited = std::collections::HashSet::new();
    while let Some(file) = frontier.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        for path in imports(&file)? {
            if !read.contains(&path) {
                read.push(path.clone());
            }
            // A corpus file the evaluator opens through this import has
            // imports of its own, and those are read the same way.
            if let Some(next) = path.strip_prefix("tests/workflow/") {
                frontier.push(next.to_owned());
            }
        }
    }
    read.sort();
    read.dedup();
    for path in &read {
        if matching_inputs(&inputs, path).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// The matcher is fnmatch, not a formality: the contracts above hold
/// only while a pattern means what the classification means, and a
/// matcher that matched nothing — or everything — would vacate both
/// while still passing.
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
    // A `*` crosses a slash: a pattern names a whole path.
    assert!(matches("tests/*", "tests/workflow/valid/tags.pkl"));
    assert!(matches("a?c", "abc"));
    assert!(!matches("a?c", "ac"));
    assert!(matches("[a-c]b", "bb"));
    assert!(matches("[!x]b", "yb"));
    assert!(!matches("[!x]b", "xb"));
    assert!(!matches("[ab", "a"));
    assert!(!matches("*.pkl", "corpus.pkl.txt"));
}

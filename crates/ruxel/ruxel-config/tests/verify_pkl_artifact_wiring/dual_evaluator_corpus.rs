//! The dual-evaluator corpus as a contract with `.velnor/config.toml`:
//! every file the target reads must be an input of the `dual-evaluator`
//! check, or a PR that changes only that file classifies the mandatory
//! `Check / dual-evaluator` job unaffected. The corpus lives in a
//! feature-gated target the pkl-less lanes skip, so an omission there
//! cannot catch itself: this target, an ordinary one of the same lane,
//! reads the corpus source rather than running it.

use super::*;

/// The fnmatch matcher the input classification means; a utility kept
/// beside this module so the contract itself stays small.
#[path = "fnmatch.rs"]
mod fnmatch;

use fnmatch::matching_inputs;

/// The Pkl text scanner the corpus contract reads sources with; a
/// utility kept beside this module so the contract itself stays small.
#[path = "scan.rs"]
mod scan;

use scan::{code, corpus_path, literal, nonrelative};

/// corpus: the secret-language suite, and the systemd suite PR #94
/// added. Each declares its own list in its own element shape — the
/// plain string, the `(path, expected)` pair — and each list is half
/// the contract: a systemd-only PR must rerun the mandatory check as
/// surely as a secret-language one.
/// The harnesses whose `EXAMPLES` arrays name the dual-evaluator
/// corpus: the secret-language suite, and the systemd suite PR #94
/// added. Each declares its own list in its own element shape — the
/// plain string, the `(path, expected)` pair — and each list is half
/// the contract: a systemd-only PR must rerun the mandatory check as
/// surely as a secret-language one.
const HARNESSES: &[(&str, &str)] = &[
    (
        "crates/ruxel/ruxel-config/tests/dual_evaluator.rs",
        "const EXAMPLES: &[&str] = &[",
    ),
    (
        "crates/ruxel/ruxel-config/tests/dual_evaluator_systemd.rs",
        "const EXAMPLES: &[(&str, bool)] = &[",
    ),
];

/// One harness's `EXAMPLES` array, resolved against `tests/workflow` the
/// way that suite's own `root()` resolves them. The array is parsed
/// structurally, element by element: a string literal is an entry, a
/// parenthesized pair is one too — its first string the path, the rest
/// of the pair, the expected verdict, carrying no path — and
/// whitespace, commas and `//` comments separate them. Anything else —
/// a named constant, a concatenation, a macro — is a loud failure,
/// never an element the parse silently skips while the invariants
/// above pass over the path it names. The terminator is part of that
/// structure (`PRRT_kwDOS3Fimc62jyW4`): the array ends at the `];` the
/// scan reaches at an element boundary — the comment branch has eaten
/// a comment's text, a `];` inside one included, long before — so a
/// comment cannot end the array early and leave later entries
/// unchecked.
fn examples(harness: &str, declaration: &str) -> Result<Vec<String>> {
    let path = repo().join(harness);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let start = text
        .find(declaration)
        .ok_or(format!("{harness} declares no EXAMPLES corpus"))?;
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
                // Only a `]` reached between elements can close the
                // array, the semicolon possibly behind whitespace; a
                // `]` that closes nothing is a loud failure.
                let semicolon = body[at + 1..]
                    .iter()
                    .position(|head| !head.is_whitespace())
                    .map(|skip| at + 1 + skip);
                if semicolon.is_none_or(|at| body[at] != ';') {
                    return Err("the EXAMPLES corpus carries a `]` that closes no array".into());
                }
                terminated = true;
                break;
            }
            '"' => examples.push(literal(&body, &mut at)?),
            '(' => {
                at += 1;
                // A pair element carries its path in the first string;
                // the rest — the expected verdict — names no file and is
                // consumed to the closing paren the text must carry.
                while matches!(body.get(at), Some(head) if head.is_whitespace()) {
                    at += 1;
                }
                if body.get(at) != Some(&'"') {
                    return Err("a paired EXAMPLES entry opens with no path".into());
                }
                let example = literal(&body, &mut at)?;
                while at < body.len() && body[at] != ')' {
                    at += 1;
                }
                if at >= body.len() {
                    return Err("a paired EXAMPLES entry carries no closing paren".into());
                }
                at += 1;
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

/// The corpus files one file reads beyond itself: the relative
/// `import`/`amends` targets, resolved against the file's own
/// directory, in either of Pkl's two forms — the statement
/// `import "path"` and the expression `import("path")`, an `amends`
/// amending the same way (`PRRT_kwDOS3Fimc62jyWw`: the evaluator
/// honours both, `eval/confined/tests.rs` among its proofs). The
/// expression is a value, taken wherever it stands — an assignment's
/// right-hand side (`hosts = import("../x.pkl").workers`, the
/// follow-up finding) as much as a line-leading clause, more than once
/// a line — while comment text never is, and nonrelative targets — a
/// URI scheme, an absolute path — never are either.
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
        // Comment text names no file the evaluator opens, so the scan
        // reads the code a tokenizer, not a substring split, sees.
        let line = code(line).trim_start();
        let mut targets = Vec::new();
        // The statement form is a clause: the line leads with it, the
        // opening quote spent in the prefix.
        for form in ["import \"", "amends \""] {
            if let Some(rest) = line.strip_prefix(form) {
                let (target, _) = rest
                    .split_once('"')
                    .ok_or(format!("{file}: an import carries no closing quote"))?;
                targets.push(target.to_owned());
            }
        }
        // The expression form is a value: it is taken wherever it
        // stands, and a line may carry more than one.
        for form in ["import(", "amends("] {
            let mut from = 0;
            while let Some(at) = line[from..].find(form) {
                from += at + form.len();
                let Some(rest) = line[from..].trim_start().strip_prefix('"') else {
                    continue;
                };
                let (target, _) = rest
                    .split_once('"')
                    .ok_or(format!("{file}: an import carries no closing quote"))?;
                targets.push(target.to_owned());
            }
        }
        for target in targets {
            if nonrelative(&target) {
                continue;
            }
            if let Some(path) = corpus_path(parent, &target) {
                imports.push(format!("tests/workflow/{path}"));
            }
        }
    }
    imports.sort();
    imports.dedup();
    Ok(imports)
}

/// `PRRT_kwDOS3Fimc62ioGG`: every workflow each dual-evaluator harness
/// names must be an input of the `dual-evaluator` check — the
/// secret-language corpus and PR #94's systemd corpus alike. The lists
/// and the input list are maintained by hand and can drift: PR #90
/// added `valid/user-missing-account.pkl` without declaring it, and a
/// PR that changed only that file classified the mandatory
/// `Check / dual-evaluator` job unaffected. The suites that would
/// notice are the ones being skipped, so the contract lives in this
/// ordinary, always-run lane.
#[test]
fn every_dual_evaluator_example_is_a_declared_input() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    for (harness, declaration) in HARNESSES {
        for example in examples(harness, declaration)? {
            let path = format!("tests/workflow/{example}");
            if !repo().join(&path).is_file() {
                return Err(format!(
                    "{harness}: EXAMPLES names {path}, which is not a committed file"
                )
                .into());
            }
            if matching_inputs(&inputs, &path).is_empty() {
                return Err(format!(
                    "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
                )
                .into());
            }
        }
    }
    Ok(())
}

/// The corpus fixtures the harnesses read directly, beyond what their
/// examples import — derived from the `root().join("...")` calls their
/// own sources make (`PRRT_kwDOS3Fimc62oWc-`): the day a harness module
/// starts reading a new fixture, that fixture is required the same
/// day, instead of a second hand-kept list drifting the way the corpus
/// list once did.
fn fixtures() -> Result<Vec<String>> {
    let base = repo().join("crates/ruxel/ruxel-config/tests");
    let mut sources = Vec::new();
    let read = |dir: &Path| -> Result<_> {
        Ok(std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?)
    };
    for entry in read(&base)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name == "dual_evaluator.rs" || name == "dual_evaluator_systemd.rs" {
            sources.push(entry.path());
        } else if name.starts_with("dual_evaluator") && entry.file_type()?.is_dir() {
            for module in read(&entry.path())? {
                sources.push(module?.path());
            }
        }
    }
    let mut fixtures = Vec::new();
    for source in sources {
        let text = std::fs::read_to_string(&source)
            .map_err(|error| format!("{}: {error}", source.display()))?;
        for rest in text.split("root().join(\"").skip(1) {
            let Some((literal, _)) = rest.split_once('"') else {
                return Err(format!(
                    "{}: a root().join carries no string literal",
                    source.display()
                )
                .into());
            };
            fixtures.push(format!("tests/workflow/{literal}"));
        }
    }
    fixtures.sort();
    fixtures.dedup();
    Ok(fixtures)
}

/// The corpus the dual-evaluator reads beyond the examples either
/// harness names, derived from what the targets actually open: the
/// fixtures the harness sources read directly, and every corpus file a
/// listed example imports by a relative path — the only way a case
/// reaches a file beyond itself.
/// `PRRT_kwDOS3Fimc62jyWq`: the walk is transitive — an example that
/// imports an aggregator (`recipes.pkl`) makes the evaluator open the
/// aggregator's own imports too, so a corpus file reached only through
/// an aggregator is still demanded — and the visited set bounds it:
/// every file is parsed once; a cycle cannot loop it.
/// `PRRT_kwDOS3Fimc62jyXrx`: nothing the suite never opens is required.
/// No example imports a recipe today, so `recipes.pkl` and its leaves
/// are not demanded; the day an example imports one, it becomes
/// required, because that is the day the suite reads it.
#[test]
fn every_corpus_file_the_dual_evaluator_reads_is_declared() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    let mut read = fixtures()?;
    let mut frontier = Vec::new();
    for (harness, declaration) in HARNESSES {
        frontier.extend(examples(harness, declaration)?);
    }
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

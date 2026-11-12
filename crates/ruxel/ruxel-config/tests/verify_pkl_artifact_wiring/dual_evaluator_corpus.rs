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

/// The dual-evaluator harness targets, derived from the `[[test]]`
/// entries Cargo gates behind the `dual-evaluator` feature
/// (`PRRT_kwDOS3Fimc62pCSX`): the feature gate is what hides those
/// targets from the pkl-less lanes, so it is also what names them — a
/// third dual-evaluator target cannot run without Cargo declaring it,
/// and a gated target that carries no `EXAMPLES` corpus this parse
/// knows is a loud failure, never a corpus quietly outside the
/// contract. Each harness declares its list in its own element shape
/// — the plain string, the `(path, expected)` pair — and each list is
/// half the contract: a systemd-only PR must rerun the mandatory
/// check as surely as a secret-language one.
fn harnesses() -> Result<Vec<(String, String)>> {
    let path = repo().join("crates/ruxel/ruxel-config/Cargo.toml");
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let manifest = text
        .parse::<DocumentMut>()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let tests = manifest
        .get("test")
        .and_then(Item::as_array_of_tables)
        .ok_or("ruxel-config's Cargo.toml declares no [[test]] targets")?;
    let declarations = [
        "const EXAMPLES: &[&str] = &[",
        "const EXAMPLES: &[(&str, bool)] = &[",
    ];
    let mut harnesses = Vec::new();
    for test in tests {
        let gated = test
            .get("required-features")
            .and_then(Item::as_array)
            .is_some_and(|features| {
                features
                    .iter()
                    .any(|feature| feature.as_str() == Some("dual-evaluator"))
            });
        if !gated {
            continue;
        }
        let name = test
            .get("name")
            .and_then(Item::as_str)
            .ok_or("a dual-evaluator [[test]] target carries no name")?;
        let path = format!("crates/ruxel/ruxel-config/tests/{name}.rs");
        let text = std::fs::read_to_string(repo().join(&path))
            .map_err(|error| format!("{path}: {error}"))?;
        let Some(declaration) = declarations
            .iter()
            .find(|declaration| text.contains(*declaration))
        else {
            return Err(format!(
                "{path} carries no EXAMPLES corpus this parse knows; a dual-evaluator \
                 target without one is a corpus outside the contract"
            )
            .into());
        };
        harnesses.push((path, (*declaration).to_owned()));
    }
    Ok(harnesses)
}

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
    // The scan reads the mask the tokenizer draws over the source —
    // string text and comments name no file the evaluator opens —
    // while each target literal is read from the source itself, at
    // the very offset the mask pointed to: a form is found in code,
    // its argument is a string either way (`PRRT_kwDOS3Fimc62pCSD`).
    let code = code(&text);
    for (line, code) in text.lines().zip(code.lines()) {
        let mut targets = Vec::new();
        for form in ["import \"", "amends \"", "import(", "amends("] {
            let mut from = 0;
            while let Some(at) = code[from..].find(form) {
                from += at + form.len();
                // The statement forms have spent their opening quote in
                // the form; the expression forms still carry it,
                // possibly behind whitespace.
                let rest = if form.ends_with('"') {
                    &line[from..]
                } else {
                    let rest = line[from..].trim_start();
                    match rest.strip_prefix('"') {
                        Some(rest) => rest,
                        None => continue,
                    }
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
    for (harness, declaration) in harnesses()? {
        for example in examples(&harness, &declaration)? {
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

/// The source files of the dual-evaluator harnesses — every gated
/// target and every module file under its sibling directory — derived
/// from the same manifest the harness set comes from. A harness module
/// is itself a file the check reads (`PRRT_kwDOS3Fimc62pCSO`): its
/// fixtures are demanded through `fixtures()`, and the module itself
/// must be declared an input the same way, or a PR changing only it
/// classifies the mandatory check unaffected.
fn harness_sources() -> Result<Vec<String>> {
    let mut sources = Vec::new();
    for (harness, _) in harnesses()? {
        sources.push(harness.clone());
        let stem = harness.trim_end_matches(".rs");
        let directory = repo().join(stem);
        if !directory.is_dir() {
            continue;
        }
        for module in std::fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?
        {
            let module = module?;
            if module.path().extension().is_some_and(|ext| ext == "rs") {
                sources.push(format!("{stem}/{}", module.file_name().to_string_lossy()));
            }
        }
    }
    Ok(sources)
}

/// The corpus fixtures the harnesses read directly, beyond what their
/// examples import — derived from the `root().join("...")` calls their
/// own sources make (`PRRT_kwDOS3Fimc62oWc-`): the day a harness module
/// starts reading a new fixture, that fixture is required the same
/// day, instead of a second hand-kept list drifting the way the corpus
/// list once did.
fn fixtures() -> Result<Vec<String>> {
    let mut fixtures = Vec::new();
    for source in harness_sources()? {
        let path = repo().join(&source);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        for rest in text.split("root().join(\"").skip(1) {
            let Some((literal, _)) = rest.split_once('"') else {
                return Err(format!("{source}: a root().join carries no string literal").into());
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
    // A harness source is a file the check reads too: an undeclared
    // module would classify the mandatory check unaffected for a PR
    // that changes only it (`PRRT_kwDOS3Fimc62pCSO`).
    for source in harness_sources()? {
        if matching_inputs(&inputs, &source).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {source} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
    let mut frontier = Vec::new();
    for (harness, declaration) in harnesses()? {
        frontier.extend(examples(&harness, &declaration)?);
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

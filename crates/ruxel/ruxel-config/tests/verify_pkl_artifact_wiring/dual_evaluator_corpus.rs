//! The dual-evaluator corpus as a contract with `.velnor/config.toml`:
//! every file the target reads must be an input of the `dual-evaluator`
//! check, or a PR that changes only that file classifies the mandatory
//! `Check / dual-evaluator` job unaffected. The corpus lives in a
//! feature-gated target the pkl-less lanes skip, so an omission there
//! cannot catch itself: this target, an ordinary one of the same lane,
//! reads the corpus source rather than running it. The invariant's
//! scope is the rule set `scan.rs` states once — imports recognized
//! only in code and interpolation regions, never literal text or
//! comments (line or block); block comments masked; only relative
//! corpus paths followed; every URI scheme and `modulepath:` excluded;
//! every discovered harness source path a declared input, and the mise
//! task executing every discovered harness — and a file outside those
//! rules is a scope question, not a defect.

use super::*;

/// The source mask both this contract and the harness discovery read
/// sources with; one scanner for the whole lane, so a rule that holds
/// for the contract's own parse holds for the discovery that follows
/// the corpus's path attributes.
#[path = "../support/pkl_mask.rs"]
mod mask;

/// The fnmatch matcher the input classification means; a utility kept
/// beside this module so the contract itself stays small.
#[path = "fnmatch.rs"]
mod fnmatch;

use fnmatch::matching_inputs;

/// The Pkl text scanner the corpus contract reads sources with; a
/// utility kept beside this module so the contract itself stays small.
#[path = "scan.rs"]
mod scan;

use scan::{declared_at, literal};

/// The harness discovery — the gated targets, their module sources,
/// their fixtures, the task that must run them; utilities kept beside
/// this module so the contract itself stays small.
#[path = "harnesses.rs"]
mod harnesses;

use harnesses::{harness_sources, harnesses};

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
    let start =
        declared_at(&text, declaration).ok_or(format!("{harness} declares no EXAMPLES corpus"))?;
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
    let harnesses = harnesses()?;
    harnesses::task_runs_every_harness(&harnesses)?;
    // The corpus the harnesses touch at runtime, from the manifest each
    // verifies against its own touched(): the examples it names and
    // every fixture a harness source opens by a literal root().join().
    let mut read = harnesses::runtime_files(&harnesses)?;
    read.sort();
    read.dedup();
    for path in &read {
        if !repo().join(path).is_file() {
            return Err(format!(
                "the corpus-files manifest names {path}, which is not a committed file"
            )
            .into());
        }
        if matching_inputs(&inputs, path).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
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
    Ok(())
}

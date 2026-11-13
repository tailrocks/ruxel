//! The corpus files this harness touches at runtime, and the manifest
//! that pins them for the wiring contract.

use super::*;

/// The relative-import closure of the corpus: the files an example or
/// fixture opens beyond itself, so a helper reached only through an
/// import is on the manifest the wiring invariant reads.
#[path = "../support/corpus_imports.rs"]
mod corpus_imports;

/// The corpus files this harness touches at runtime, as a value the
/// wiring contract can read: every example it names (each opened by
/// `root().join(name)`), every fixture a harness source opens by a
/// literal `root().join(...)`, and every file those open by a
/// relative import, transitively, because the evaluator reads those
/// too and a change to one alone must not classify the check
/// unaffected. A `root().join` whose argument is not a string literal
/// is a loud failure here, not a silently unlisted file.
/// The `Result` stays: the sources are `include_str!`ed today, so the
/// unhappy path looks unreachable to the lint, but a future fixture
/// spelled without a literal must keep failing loudly.
fn touched() -> Result<Vec<String>> {
    let mut files: Vec<String> = EXAMPLES
        .iter()
        .map(|(example, _)| format!("tests/workflow/{example}"))
        .collect();
    for source in [include_str!("../dual_evaluator_systemd.rs")] {
        for rest in source.split("root().join(\"").skip(1) {
            let Some((literal, _)) = rest.split_once('"') else {
                return Err(
                    "a harness root().join carries no string literal; name the fixture in touched()"
                        .into(),
                );
            };
            files.push(format!("tests/workflow/{literal}"));
        }
    }
    corpus_imports::expand(&mut files)?;
    files.sort();
    files.dedup();
    Ok(files)
}

/// The committed corpus-files manifest must say exactly what this
/// harness touches: the wiring invariant demands its every line a
/// declared input, so a manifest that drifts from the runtime truth
/// would enforce the wrong list.
#[test]
fn the_corpus_files_manifest_matches_this_harness() -> Result<()> {
    let manifest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/testdata/dual_evaluator_corpus_files.tsv"
    ))
    .expect("the corpus-files manifest is committed");
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in manifest.lines() {
        if let Some(file) = line.strip_prefix("  ") {
            sections
                .last_mut()
                .expect("a manifest file precedes its section header")
                .1
                .push(file);
        } else if !line.starts_with('#') && !line.is_empty() {
            sections.push((line, Vec::new()));
        }
    }
    let (_, listed) = sections
        .iter()
        .find(|(harness, _)| *harness == "dual_evaluator_systemd")
        .expect("the manifest carries no dual_evaluator_systemd section");
    let touched = touched()?;
    assert_eq!(
        listed, &touched,
        "the manifest and this harness disagree; regenerate \
         testdata/dual_evaluator_corpus_files.tsv from touched()"
    );
    Ok(())
}

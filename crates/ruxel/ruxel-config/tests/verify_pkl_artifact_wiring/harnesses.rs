//! The dual-evaluator harness discovery: which targets Cargo gates
//! behind the `dual-evaluator` feature, the module sources beneath
//! them, the fixtures their own reads name, and the mise task that
//! must execute every one of them. Kept beside `dual_evaluator_corpus.rs`
//! — utilities, not the contract itself.

use super::super::*;
use super::scan::declared_at;

/// The dual-evaluator harness targets, derived from the `[[test]]`
/// entries Cargo gates behind the `dual-evaluator` feature
/// (`PRRT_kwDOS3Fimc62pCSX`): the feature gate is what hides those
/// targets from the pkl-less lanes, so it is also what names them — a
/// third dual-evaluator target cannot run without Cargo declaring it,
/// and a gated target that carries no `EXAMPLES` corpus this parse
/// knows is a loud failure, never a corpus quietly outside the
/// contract.
pub(super) fn harnesses() -> Result<Vec<(String, String)>> {
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
            .find(|declaration| declared_at(&text, declaration).is_some())
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

/// The Rust sources under one directory, at any depth
/// (`PRRT_kwDOS3Fimc62plX_l`): a `mod foo;` backed by `foo/mod.rs`
/// nests, and every nested harness source is a harness source.
fn sources_under(stem: &str, sources: &mut Vec<String>) -> Result<()> {
    let directory = repo().join(stem);
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            sources_under(&format!("{stem}/{name}"), sources)?;
        } else if entry.path().extension().is_some_and(|ext| ext == "rs") {
            sources.push(format!("{stem}/{name}"));
        }
    }
    Ok(())
}

/// The source files of the dual-evaluator harnesses — every gated
/// target and every module file under its sibling directory, at any
/// depth (`PRRT_kwDOS3Fimc62pCSO`): a harness module is itself a file
/// the check reads, and must be declared an input the way its corpus
/// files are, or a PR changing only it classifies the mandatory check
/// unaffected.
pub(super) fn harness_sources() -> Result<Vec<String>> {
    let mut sources = Vec::new();
    let mut frontier: Vec<String> = harnesses()?
        .into_iter()
        .map(|(harness, _)| harness)
        .collect();
    while let Some(source) = frontier.pop() {
        if sources.contains(&source) {
            continue;
        }
        sources.push(source.clone());
        sources_under(source.trim_end_matches(".rs"), &mut sources)?;
        // A `#[path = "..."]`-attributed module may point anywhere; the
        // harness reads it, so the contract follows it there.
        let path = repo().join(&source);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        for rest in text.split("#[path = \"").skip(1) {
            let Some((literal, _)) = rest.split_once('"') else {
                return Err(
                    format!("{source}: a #[path] attribute carries no closing quote").into(),
                );
            };
            let directory = source
                .rsplit_once('/')
                .map(|(head, _)| head)
                .ok_or("a harness source has no directory")?;
            // Lexical resolution, the way #[path] itself resolves: `..`
            // pops, `.` drops, and the result names one canonical path.
            let mut parts: Vec<&str> = Vec::new();
            for piece in Path::new(directory)
                .iter()
                .chain(Path::new(literal).iter())
                .map(|piece| piece.to_str())
            {
                let Some(piece) = piece else {
                    continue;
                };
                match piece {
                    "." => {}
                    ".." => {
                        parts.pop();
                    }
                    part => parts.push(part),
                }
            }
            if parts.first().is_none_or(|first| *first == "..") {
                return Err(
                    format!("{source}: #[path = \"{literal}\"] leaves the repository").into(),
                );
            }
            frontier.push(parts.join("/"));
        }
    }
    sources.sort();
    Ok(sources)
}

/// The corpus files the harnesses touch at runtime, from the
/// committed manifest each harness verifies against its own `touched()`:
/// the examples it names and every fixture a harness source opens by a
/// literal `root().join(...)`. The manifest is the runtime truth as a
/// value — no Pkl source is parsed to predict it — and every section
/// must name a discovered harness, every harness a section, or the
/// manifest enforces the wrong list.
pub(super) fn runtime_files(harnesses: &[(String, String)]) -> Result<Vec<String>> {
    let path = repo().join("crates/ruxel/ruxel-config/testdata/dual_evaluator_corpus_files.tsv");
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(file) = line.strip_prefix("  ") {
            sections
                .last_mut()
                .ok_or("the manifest lists a file before any section header")?
                .1
                .push(file.to_owned());
        } else if !line.starts_with('#') && !line.is_empty() {
            sections.push((line.to_owned(), Vec::new()));
        }
    }
    let mut files = Vec::new();
    for (harness, _) in harnesses {
        let name = match harness.rsplit_once('/') {
            Some((_, name)) => name,
            None => harness,
        }
        .trim_end_matches(".rs");
        let (_, listed) = sections
            .iter()
            .find(|(header, _)| header == name)
            .ok_or(format!(
                "the corpus-files manifest carries no {name} section"
            ))?;
        files.extend(listed.iter().cloned());
    }
    for (header, _) in &sections {
        let known = harnesses.iter().any(|(harness, _)| {
            harness
                .rsplit_once('/')
                .map(|(_, name)| name)
                .is_some_and(|name| name.trim_end_matches(".rs") == header)
        });
        if !known {
            return Err(format!(
                "the corpus-files manifest carries a {header} section no gated harness backs"
            )
            .into());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The mise `dual-evaluator` task is CI's mandatory runner for the
/// gated targets; every harness the contract discovers must appear in
/// its commands (`PRRT_kwDOS3Fimc62plYG`): a harness wired into the
/// corpus the task never executes is a corpus nobody checks, and the
/// contract says so rather than passing over it.
pub(super) fn task_runs_every_harness(harnesses: &[(String, String)]) -> Result<()> {
    let path = repo().join("mise.toml");
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mise = text
        .parse::<DocumentMut>()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let commands = mise
        .get("tasks")
        .and_then(Item::as_table)
        .and_then(|tasks| tasks.get("dual-evaluator"))
        .and_then(Item::as_table)
        .and_then(|task| task.get("run"))
        .and_then(Item::as_array)
        .ok_or("mise.toml declares no dual-evaluator task with a run list")?;
    let commands: Vec<&str> = commands
        .iter()
        .filter_map(|command| command.as_str())
        .collect();
    for (harness, _) in harnesses {
        let name = match harness.rsplit_once('/') {
            Some((_, name)) => name,
            None => harness,
        }
        .trim_end_matches(".rs");
        // Each harness needs its own exact command: `--test dual_evaluator`
        // is a prefix of `--test dual_evaluator_systemd`, and a substring
        // check would let the systemd command stand in for the main one.
        let executed = commands.iter().any(|command| {
            command
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["--test", name])
        });
        if !executed {
            return Err(format!(
                "the mise dual-evaluator task never runs {name} (no `--test {name}` argument); \
                 a harness the mandatory task does not execute is a corpus nobody checks"
            )
            .into());
        }
    }
    Ok(())
}

//! `schema/Ruxel.pkl` is the only copy of the schema in this repository.
//!
//! Configuration imports `modulepath:/ruxel/Ruxel.pkl`, which the binary
//! serves. A second copy, or code that stages one next to a fixture, drifts
//! from the binary and is refused at run time; this test catches it first.

use std::path::{Path, PathBuf};

const CANONICAL: &str = "schema/Ruxel.pkl";
/// Files that may name the canonical path: the embedding, its tests, and
/// the plan coverage check, which parses the canonical file in place.
/// Markdown may always name it, and so may the CI cache-key input lists.
const MAY_NAME_CANONICAL: &[&str] = &[
    "crates/ruxel/ruxel-config/src/schema.rs",
    "crates/ruxel/ruxel-config/src/schema/tests.rs",
    "crates/ruxel/ruxel-config/tests/eval.rs",
    "crates/ruxel/ruxel-config/tests/single_schema.rs",
    "tools/acceptance-plan/src/coverage/mod.rs",
    ".velnor/config.toml",
    ".github/workflows/ci.yml",
    ".github/workflows/state-seal.yml",
];
/// `.ruxel` holds the ignored `ruxel schema export` the README documents.
const SKIPPED: &[&str] = &[".git", "target", ".cache", ".ruxel"];

fn files(directory: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !SKIPPED.iter().any(|name| entry.file_name() == *name) {
                files(&path, found)?;
            }
        } else if kind.is_file() || kind.is_symlink() {
            found.push(path);
        }
    }
    Ok(())
}

fn violations(root: &Path) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    files(root, &mut found)?;
    let mut violations = Vec::new();
    for path in found {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == CANONICAL {
            continue;
        }
        if path.is_symlink() {
            let canonical = root.join(CANONICAL).canonicalize()?;
            // A link to the schema, or to a directory that contains it.
            if path.file_name().is_some_and(|name| name == "Ruxel.pkl")
                || path
                    .canonicalize()
                    .is_ok_and(|target| canonical.starts_with(target))
            {
                violations.push(format!(
                    "{relative}: a link to the schema; import \"modulepath:/ruxel/Ruxel.pkl\" instead"
                ));
            }
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if path.file_name().is_some_and(|name| name == "Ruxel.pkl")
            || text.starts_with("module Ruxel\n")
        {
            violations.push(format!(
                "{relative}: a copy of the schema; import \"modulepath:/ruxel/Ruxel.pkl\" instead"
            ));
        } else if text.contains(CANONICAL)
            && path.extension().is_none_or(|extension| extension != "md")
            && !MAY_NAME_CANONICAL.contains(&relative.as_str())
        {
            violations.push(format!(
                "{relative}: names {CANONICAL}; fixtures import \"modulepath:/ruxel/Ruxel.pkl\", which the ruxel binary serves, so nothing copies or stages the schema"
            ));
        }
    }
    violations.sort();
    Ok(violations)
}

#[test]
fn the_schema_has_exactly_one_copy() -> std::io::Result<()> {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
    let violations = violations(root)?;
    assert!(
        violations.is_empty(),
        "schema copies or staging found:\n{}",
        violations.join("\n")
    );
    Ok(())
}

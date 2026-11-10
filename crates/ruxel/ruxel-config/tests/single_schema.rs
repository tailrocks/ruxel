//! Every Ruxel Pkl module exists once in this repository: under
//! `schema/ruxel/`, which the binary embeds and serves at `modulepath:/ruxel/`.
//!
//! A second copy, a link to one, or code that stages one next to a fixture
//! drifts from the binary and is refused at run time; this test catches it
//! first. It fails on any other `.pkl` file whose file name or module name
//! matches an embedded module, on links into `schema/ruxel/`, and on code or
//! configuration outside the embedding that names `schema/ruxel/`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const EMBEDDED: &str = "schema/ruxel/";
/// Files that may name the embedded directory: the embedding, its tests, and
/// the plan coverage check, which parses the engine schema in place.
/// Markdown may always name it, and so may the CI cache-key input lists.
const MAY_NAME_EMBEDDED: &[&str] = &[
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

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The name in the first `module` or `open module` line, if any.
fn module_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line
            .strip_prefix("open module ")
            .or_else(|| line.strip_prefix("module "))?;
        Some(rest.trim().to_owned())
    })
}

fn violations(root: &Path) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    files(root, &mut found)?;
    let embedded: Vec<&PathBuf> = found
        .iter()
        .filter(|path| !path.is_symlink() && relative(root, path).starts_with(EMBEDDED))
        .collect();
    let file_names: BTreeSet<String> = embedded
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    let module_names: BTreeSet<String> = embedded
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| module_name(&text))
        .collect();
    let embedded_dir = root.join(EMBEDDED).canonicalize()?;
    let mut violations = Vec::new();
    for path in &found {
        let relative = relative(root, path);
        if relative.starts_with(EMBEDDED) && !path.is_symlink() {
            continue;
        }
        let same_file = path
            .file_name()
            .is_some_and(|name| file_names.contains(&*name.to_string_lossy()));
        let is_pkl = path.extension().is_some_and(|extension| extension == "pkl");
        if path.is_symlink() {
            // A link to a module, or to a directory that contains them.
            if (is_pkl && same_file)
                || path.canonicalize().is_ok_and(|target| {
                    target.starts_with(&embedded_dir) || embedded_dir.starts_with(&target)
                })
            {
                violations.push(format!(
                    "{relative}: a link to an embedded Ruxel module; import it from \"modulepath:/ruxel/\" instead"
                ));
            }
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let same_module = module_name(&text).is_some_and(|name| module_names.contains(&name));
        if is_pkl && (same_file || same_module) {
            violations.push(format!(
                "{relative}: a copy of an embedded Ruxel module; import it from \"modulepath:/ruxel/\" instead"
            ));
        } else if text.contains(EMBEDDED)
            && path.extension().is_none_or(|extension| extension != "md")
            && !MAY_NAME_EMBEDDED.contains(&relative.as_str())
        {
            violations.push(format!(
                "{relative}: names {EMBEDDED}; fixtures import \"modulepath:/ruxel/...\", which the ruxel binary serves, so nothing copies or stages a Ruxel module"
            ));
        }
    }
    violations.sort();
    Ok(violations)
}

#[test]
fn every_ruxel_module_has_exactly_one_copy() -> std::io::Result<()> {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
    let violations = violations(root)?;
    assert!(
        violations.is_empty(),
        "Ruxel module copies or staging found:\n{}",
        violations.join("\n")
    );
    Ok(())
}

//! Evaluates the workflow language corpus in `tests/workflow`.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn corpus(kind: &str) -> Result<Vec<PathBuf>> {
    let directory = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .join(kind);
    let mut files = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.extension().is_some_and(|extension| extension == "pkl"));
    files.sort();
    assert!(!files.is_empty(), "no {kind} workflows");
    Ok(files)
}

/// The text the error must contain: `// expect-pklr:` when pklr's wording
/// differs from Pkl's, otherwise `// expect:`.
fn expected(source: &str) -> Option<&str> {
    let line = |prefix: &str| source.lines().find_map(|line| line.strip_prefix(prefix));
    line("// expect-pklr: ").or_else(|| line("// expect: "))
}

#[test]
fn valid_workflows_are_admitted_with_plays() -> Result {
    for path in corpus("valid")? {
        let config = ruxel_config::eval_config(&path, &[])
            .map_err(|error| format!("{}: {error}", path.display()))?;
        // Workflow.pkl renders the engine configuration as the module output.
        let plays = config["plays"].as_array().ok_or("no plays")?;
        assert!(!plays.is_empty(), "{}: no plays", path.display());
    }
    Ok(())
}

#[test]
fn invalid_workflows_fail_with_their_message() -> Result {
    for path in corpus("invalid")? {
        let source = std::fs::read_to_string(&path)?;
        let expected =
            expected(&source).ok_or_else(|| format!("{}: no expect line", path.display()))?;
        let Err(error) = ruxel_config::eval_config(&path, &[]) else {
            panic!("{}: evaluated, expected {expected:?}", path.display());
        };
        assert!(
            error.to_string().contains(expected),
            "{}: expected {expected:?} in {error}",
            path.display()
        );
    }
    Ok(())
}

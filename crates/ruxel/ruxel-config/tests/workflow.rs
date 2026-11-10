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

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
}

fn valid(name: &str) -> Result<serde_json::Value> {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow/valid"
    ))
    .join(name);
    Ok(ruxel_config::eval_config(&path, &[])?)
}

#[test]
fn captures_run_in_check_and_diff() -> Result {
    let config = valid("shared-capture.pkl")?;
    let mut overridden: Vec<&str> = tasks(&config)
        .into_iter()
        .filter(|task| !task["checkMode"].is_null())
        .map(|task| {
            assert_eq!(task["checkMode"], false, "{task:#}");
            task["name"].as_str().unwrap_or_default()
        })
        .collect();
    overridden.sort_unstable();
    assert_eq!(
        overridden,
        [
            "Read the control plane CA public key hash",
            "Read the release after the upgrade",
            "Read the release before the upgrade",
        ]
    );
    Ok(())
}

#[test]
fn apt_sources_name_architectures_only_when_set() -> Result {
    let config = valid("apt-repositories.pkl")?;
    let content = |dest: &str| {
        tasks(&config)
            .into_iter()
            .find(|task| task["operation"]["dest"] == dest)
            .and_then(|task| task["operation"]["content"].as_str().map(str::to_owned))
    };
    let native = content("/etc/apt/sources.list.d/native.sources").ok_or("no native sources")?;
    let pinned = content("/etc/apt/sources.list.d/pinned.sources").ok_or("no pinned sources")?;
    assert!(!native.contains("Architectures:"), "{native}");
    assert!(pinned.contains("Architectures: arm64\n"), "{pinned}");
    Ok(())
}

#[test]
fn edge_values_lower_with_their_guards() -> Result {
    let config = valid("edge-values.pkl")?;
    assert_eq!(
        config["inventory"]["hosts"]["link-local"]["address"],
        "fe80::1%eth0"
    );
    let all = tasks(&config);
    let named = |name: &str| {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| format!("no task {name:?}"))
    };
    // Literal text that resembles a run-time value stays literal.
    let motd = &all[named("Write /etc/motd")?];
    assert_eq!(
        motd["operation"]["content"],
        "literal __ruxel.inventoryHost__ text\n"
    );
    // The account probe runs first, in check mode too, and changes nothing.
    let probe = named("Check that account root exists")?;
    assert_eq!(probe + 1, named("Set root login shell to /bin/zsh")?);
    assert_eq!(all[probe]["checkMode"], false);
    assert_eq!(all[probe]["changedWhen"]["value"], false);
    // A `creates` skip reports no change even though its output lacks the marker.
    let init = &all[named("Run /usr/local/bin/app-init")?];
    assert_eq!(init["changedWhen"]["function"], "and");
    assert_eq!(
        init["changedWhen"]["args"][0]["path"],
        serde_json::json!(["result", "changed"])
    );
    Ok(())
}

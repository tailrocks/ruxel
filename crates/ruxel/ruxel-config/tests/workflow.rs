//! Evaluates the workflow language corpus in `tests/workflow`.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

/// Cases import the shared inventory and recipes from the corpus root, so
/// that is the directory they may read.
fn options() -> ruxel_config::EvalOptions {
    ruxel_config::EvalOptions {
        root_dir: Some(root()),
        ..ruxel_config::EvalOptions::default()
    }
}

fn corpus(kind: &str) -> Result<Vec<PathBuf>> {
    let directory = root().join(kind);
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
        let config = ruxel_config::eval_config(&path, &options())
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
        let Err(error) = ruxel_config::eval_config(&path, &options()) else {
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
    Ok(ruxel_config::eval_config(
        &root().join("valid").join(name),
        &options(),
    )?)
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
    // The keyring directory exists before the key is written into it.
    let all = tasks(&config);
    let position = |name: &str| all.iter().position(|task| task["name"] == name);
    let directory = position("Create /etc/apt/keyrings").ok_or("no keyrings directory")?;
    assert_eq!(all[directory]["operation"]["path"], "/etc/apt/keyrings");
    assert_eq!(all[directory]["operation"]["state"], "directory");
    assert_eq!(all[directory]["register"], "aptKeyrings");
    // Root's, so nobody else can replace a key.
    for (field, value) in [("mode", "0755"), ("owner", "root"), ("group", "root")] {
        assert_eq!(all[directory]["operation"][field], value, "{field}");
    }
    // One task for the stage, before the first key.
    assert_eq!(
        all.iter()
            .filter(|task| task["name"] == "Create /etc/apt/keyrings")
            .count(),
        1
    );
    assert_eq!(
        position("Install native apt signing key"),
        Some(directory + 1)
    );
    // A step reacting to a repository sees its source, key and the directory.
    let watcher = &all[position("Run apt-get update").ok_or("no watcher")?];
    let condition = watcher["when"].to_string();
    let key = all[directory + 1]["register"]
        .as_str()
        .ok_or("key not registered")?;
    let base = key.strip_suffix("_key").ok_or("key register name")?;
    for result in [base, key, "aptKeyrings"] {
        assert!(
            condition.contains(&format!("\"{result}\"")),
            "{result}: {condition}"
        );
    }
    assert!(!native.contains("Architectures:"), "{native}");
    assert!(pinned.contains("Architectures: arm64\n"), "{pinned}");
    Ok(())
}

#[test]
fn edge_values_lower_with_their_guards() -> Result {
    let config = valid("edge-values.pkl")?;
    for (host, address) in [
        ("link-local", "fe80::1%eth0"),
        ("mapped", "::ffff:192.0.2.7"),
        ("documentation", "2001:db8::42"),
    ] {
        assert_eq!(config["inventory"]["hosts"][host]["address"], address);
    }
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
    // A command that ran, or would run in check and diff (`skipped`), is a
    // change, so the steps that react to it see one there too.
    let ran = serde_json::json!({"kind": "call", "function": "or", "args": [
        {"kind": "ref", "path": ["result", "changed"]},
        {"kind": "call", "function": "default", "args": [
            {"kind": "ref", "path": ["result", "skipped"]},
            {"kind": "literal", "value": false},
        ]},
    ]});
    let reload = &all[named("Run /usr/local/bin/app-reload")?];
    assert_eq!(reload["changedWhen"], ran);
    // The `creates` probe follows links, like the command's own guard.
    let probe = all
        .iter()
        .find(|task| task["name"] == "Check whether /var/lib/app/migrated exists")
        .ok_or("no creates probe")?;
    assert_eq!(probe["operation"]["follow"], true);
    // A `creates` skip reports no change even though its output lacks the marker.
    let init = &all[named("Run /usr/local/bin/app-init")?];
    assert_eq!(init["changedWhen"]["function"], "and");
    assert_eq!(init["changedWhen"]["args"][0], ran);
    Ok(())
}

/// A workflow evaluated with an exported copy of `Workflow.pkl`, of
/// `internal/Lowering.pkl`, or of both, still reaches the served engine
/// schema. Admission must refuse it all the same.
#[test]
fn copies_of_the_language_modules_are_refused() -> Result {
    let scratch =
        std::env::temp_dir().join(format!("ruxel-language-copies-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&scratch));
    for (path, text) in ruxel_config::schema::exported_modules() {
        let file = scratch.join(format!("export{path}"));
        std::fs::create_dir_all(file.parent().ok_or("no parent")?)?;
        std::fs::write(file, text)?;
    }
    let copied_lowering = "import \"modulepath:/ruxel/internal/Lowering.pkl\" as Lowering";
    let workflow = std::fs::read_to_string(scratch.join("export/ruxel/Workflow.pkl"))?;
    assert!(workflow.contains(copied_lowering));
    // A Workflow.pkl copy that uses the copied Lowering.pkl.
    std::fs::write(
        scratch.join("export/ruxel/WorkflowWithCopiedLowering.pkl"),
        workflow.replace(
            copied_lowering,
            "import \"internal/Lowering.pkl\" as Lowering",
        ),
    )?;
    let body = "\nstages { new Stage { title = \"t\"; hosts { new Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new Timezone { name = \"UTC\" } } } }\n";
    for (name, amends) in [
        ("served.pkl", "modulepath:/ruxel/Workflow.pkl"),
        ("workflow-copy.pkl", "export/ruxel/Workflow.pkl"),
        (
            "both-copies.pkl",
            "export/ruxel/WorkflowWithCopiedLowering.pkl",
        ),
    ] {
        let root = scratch.join(name);
        std::fs::write(&root, format!("amends \"{amends}\"\n{body}"))?;
        let result = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default());
        if name == "served.pkl" {
            result?;
        } else {
            let error = result
                .err()
                .ok_or_else(|| format!("{name}: admitted"))?
                .to_string();
            assert!(
                error.contains("a copy of this ruxel's schema"),
                "{name}: {error}"
            );
        }
    }
    // Stages and hosts built from a copy of Ruxel.pkl around served steps.
    let copied = "import \"export/ruxel/Ruxel.pkl\" as copied\n";
    for (name, stages) in [
        (
            "copied-stage.pkl",
            "stages { new copied.Stage { title = \"t\"; hosts { new Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new Timezone { name = \"UTC\" } } } }",
        ),
        (
            "copied-host.pkl",
            "stages { new Stage { title = \"t\"; hosts { new copied.Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new Timezone { name = \"UTC\" } } } }",
        ),
    ] {
        let root = scratch.join(name);
        std::fs::write(
            &root,
            format!("amends \"modulepath:/ruxel/Workflow.pkl\"\n{copied}{stages}\n"),
        )?;
        let error = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())
            .err()
            .ok_or_else(|| format!("{name}: admitted"))?
            .to_string();
        assert!(
            error.contains("comes from a copy of Ruxel.pkl"),
            "{name}: {error}"
        );
    }
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}

/// The validator reads a candidate only root can change, whatever the
/// file's own owner and mode.
#[test]
fn validator_candidates_belong_to_root() -> Result {
    let config = valid("composition.pkl")?;
    let staged = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Stage /etc/haproxy/haproxy.cfg")
        .ok_or("no staging task")?;
    for (field, value) in [("mode", "0600"), ("owner", "root"), ("group", "root")] {
        assert_eq!(staged["operation"][field], value, "{field}");
    }
    Ok(())
}

//! Evaluates the workflow language corpus in `tests/workflow`.
//!
//! The secret-bearing workflows of the corpus are evaluated in
//! `workflow_secrets.rs`.

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
    // The user step lowers to the engine's user operation alone: no
    // pre-check reads /etc/passwd before it.
    let user = named("Manage account root")?;
    assert_eq!(all[user]["operation"]["kind"], "user");
    assert_eq!(all[user]["operation"]["name"], "root");
    assert_eq!(all[user]["operation"]["shell"], "/bin/zsh");
    assert!(named("Check that account root exists").is_err());
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
    // The `creates` probes follow links: a dangling link does not count as
    // the path existing. The guard is a `when` condition on the controller,
    // so a skipped action resolves no secret environment, and the command
    // carries the path for its own recheck at execution time, beside the
    // `onChangeOf` result that overrides the guard.
    let position = |name: &str| all.iter().position(|task| task["name"] == name);
    for (probe_name, action_name, path, function) in [
        (
            "Check whether /var/lib/app/migrated exists",
            "Run /usr/local/bin/app-migrate",
            "/var/lib/app/migrated",
            "or",
        ),
        (
            "Check whether /var/lib/app/initialized exists",
            "Run /usr/local/bin/app-init",
            "/var/lib/app/initialized",
            "not",
        ),
    ] {
        let probe = position(probe_name).ok_or("no creates probe")?;
        let action = position(action_name).ok_or("no guarded action")?;
        assert_eq!(probe + 1, action);
        assert_eq!(all[probe]["operation"]["follow"], true);
        assert_eq!(all[action]["operation"]["creates"], path);
        assert_eq!(all[action]["when"]["function"], function);
        assert_eq!(all[action]["when"]["args"][0]["path"][0], "results");
        // The agent's recheck runs the same guard as the `when`: the watched
        // step's change, which is the gate's first disjunct when there is one.
        let watched = &all[action]["operation"]["run_if_changed"];
        if function == "or" {
            assert_eq!(*watched, all[action]["when"]["args"][0], "{action_name}");
        } else {
            assert!(watched.is_null(), "{action_name}");
        }
    }
    // A `creates` recheck skip reports no change even though its output lacks
    // the marker; every other skip (check and diff) still reports a pending one.
    let init = &all[named("Run /usr/local/bin/app-init")?];
    assert_eq!(init["changedWhen"]["function"], "and");
    assert_eq!(init["changedWhen"]["args"][0], ran_except_recheck());
    Ok(())
}

/// The `changedWhen` of a `creates` action: the command ran, or would run in
/// check and diff (`skipped`), except when the agent's own recheck skipped it
/// because the path exists now — that skip reports no change.
fn ran_except_recheck() -> serde_json::Value {
    serde_json::json!({"kind": "call", "function": "or", "args": [
        {"kind": "ref", "path": ["result", "changed"]},
        {"kind": "call", "function": "and", "args": [
            {"kind": "call", "function": "default", "args": [
                {"kind": "ref", "path": ["result", "skipped"]},
                {"kind": "literal", "value": false},
            ]},
            {"kind": "call", "function": "ne", "args": [
                {"kind": "call", "function": "default", "args": [
                    {"kind": "ref", "path": ["result", "skip", "cause"]},
                    {"kind": "literal", "value": ""},
                ]},
                {"kind": "literal", "value": "creates_exists"},
            ]},
        ]},
    ]})
}

/// A missing account is not a failure: the step lowers to the engine's user
/// operation alone, which creates the account.
#[test]
fn a_missing_account_lowers_to_the_user_operation() -> Result {
    let config = valid("user-missing-account.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 1, "{all:#?}");
    let user = &all[0];
    assert_eq!(user["name"], "Manage account ruxel-app");
    assert_eq!(user["operation"]["kind"], "user");
    assert_eq!(user["operation"]["name"], "ruxel-app");
    assert_eq!(user["operation"]["shell"], "/bin/zsh");
    assert!(
        all.iter()
            .all(|task| task["operation"]["kind"] != "command"),
        "{all:#?}"
    );
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

/// File names have at most 255 bytes, so a long name's candidate is hashed.
#[test]
fn validator_candidates_fit_a_file_name() -> Result {
    let config = valid("long-file-name.pkl")?;
    let staged = tasks(&config)
        .into_iter()
        .find(|task| {
            task["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("Stage /etc/"))
        })
        .ok_or("no staging task")?;
    let dest = staged["operation"]["dest"].as_str().ok_or("no dest")?;
    let name = dest.rsplit('/').next().ok_or("no name")?;
    assert!(name.len() <= 255, "{} bytes: {name}", name.len());
    assert!(
        name.starts_with('.') && name.ends_with(".ruxel-candidate"),
        "{name}"
    );
    Ok(())
}

/// The `results.…` names an `onChangeOf` reaction reads.
fn watched_results(expr: &serde_json::Value) -> Vec<String> {
    let nested = |key: &str| {
        expr[key]
            .as_array()
            .map(|items| items.iter().flat_map(watched_results).collect())
            .unwrap_or_default()
    };
    match expr["kind"].as_str() {
        Some("ref") => expr["path"]
            .as_array()
            .filter(|path| {
                path.first().and_then(|v| v.as_str()) == Some("results") && path.len() >= 2
            })
            .map(|path| vec![path[1].as_str().unwrap_or_default().to_owned()])
            .unwrap_or_default(),
        Some("call") => nested("args"),
        Some("list") => nested("items"),
        _ => Vec::new(),
    }
}

/// A file declared with `600` and reconstructed with `0600` is one
/// identity: both reloads react to the file step's result, and every mode
/// lowers in its canonical four-digit form.
#[test]
fn file_mode_spellings_are_one_identity_and_lower_canonically() -> Result {
    let config = valid("file-mode-spellings.pkl")?;
    let all = tasks(&config);
    let file = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/etc/app.conf")
        .ok_or("no file task")?;
    assert_eq!(file["operation"]["mode"], "0600");
    let directory = all
        .iter()
        .find(|task| task["operation"]["path"] == "/etc/app")
        .ok_or("no directory task")?;
    assert_eq!(directory["operation"]["mode"], "0755");
    let watchers: Vec<Vec<String>> = all
        .iter()
        .filter(|task| {
            task["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("Reload "))
        })
        .map(|task| watched_results(&task["when"]))
        .collect();
    assert_eq!(watchers.len(), 2, "both reloads reacted: {watchers:?}");
    assert!(watchers.iter().all(|ids| ids.len() == 1), "{watchers:?}");
    assert_eq!(watchers[0], watchers[1], "one identity: {watchers:?}");
    Ok(())
}

/// An action reconstructed with its environment pairs in another order is
/// one identity: both restarts react to the reload step's result.
#[test]
fn action_environment_order_is_one_identity() -> Result {
    let config = valid("action-environment-order.pkl")?;
    let watchers: Vec<Vec<String>> = tasks(&config)
        .into_iter()
        .filter(|task| {
            task["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("Restart "))
        })
        .map(|task| watched_results(&task["when"]))
        .collect();
    assert_eq!(watchers.len(), 2, "both restarts reacted: {watchers:?}");
    assert!(watchers.iter().all(|ids| ids.len() == 1), "{watchers:?}");
    assert_eq!(watchers[0], watchers[1], "one identity: {watchers:?}");
    Ok(())
}

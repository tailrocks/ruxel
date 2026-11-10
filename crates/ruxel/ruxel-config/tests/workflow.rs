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

/// A secret in file content lowers to an engine lookup inside the content
/// concat, and the task that writes it is never logged.
#[test]
fn secret_file_content_becomes_a_lookup_and_is_never_logged() -> Result {
    let config = valid("secret-file.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Write /etc/lightdash/lightdash.env")
        .ok_or("no secret file task")?;
    assert_eq!(task["noLog"], serde_json::json!(true));
    assert_eq!(task["operation"]["mode"], "0600");
    let lookup = serde_json::json!({
        "kind": "lookup",
        "provider": "fnox",
        "key": "LIGHTDASH_SECRET",
        "argv": [],
    });
    assert_eq!(
        task["operation"]["content"],
        serde_json::json!({
            "kind": "call",
            "function": "concat",
            "args": [
                {"kind": "literal", "value": "LIGHTDASH_SECRET="},
                lookup,
                {"kind": "literal", "value": "\n"},
            ],
        })
    );
    Ok(())
}

/// A secret environment value lowers to a lookup (a joined pair to a concat
/// of two), a static value stays a literal, and only the task with the
/// secret is no-log.
#[test]
fn secret_environment_values_become_lookups() -> Result {
    let config = valid("secret-environment.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Refresh the bootstrap token")
        .ok_or("no bootstrap task")?;
    assert_eq!(task["noLog"], serde_json::json!(true));
    let environment = task["environment"].as_object().ok_or("no environment")?;
    assert_eq!(
        environment["KUBECONFIG"],
        serde_json::json!({"kind": "literal", "value": "/etc/kubernetes/admin.conf"})
    );
    assert_eq!(
        environment["BOOTSTRAP_TOKEN"],
        serde_json::json!({
            "kind": "call",
            "function": "concat",
            "args": [
                {"kind": "lookup", "provider": "fnox", "key": "KUBEADM_TOKEN_ID", "argv": []},
                {"kind": "literal", "value": "."},
                {"kind": "lookup", "provider": "fnox", "key": "KUBEADM_TOKEN_SECRET", "argv": []},
            ],
        })
    );
    Ok(())
}

/// A secret-bearing file's validation stages a temporary candidate: the
/// stage and validate tasks sit in a block whose `always` removes the
/// candidate, so a failing validator leaves no resolved plaintext on the
/// host. A file without secrets keeps the persistent candidate instead
/// (`validator_candidates_belong_to_root`).
#[test]
fn secret_file_validation_removes_its_temporary_candidate() -> Result {
    let config = valid("secret-file-validate.pkl")?;
    let all = tasks(&config);
    let block = all
        .iter()
        .find(|task| task["block"].as_array().is_some_and(|b| !b.is_empty()))
        .ok_or("validation is not a block")?;
    assert_eq!(block["name"], "Validate /etc/app/tls.env");
    assert!(block["operation"].is_null(), "{}", block["name"]);
    assert_eq!(block["noLog"], serde_json::json!(true));
    let inner = block["block"].as_array().ok_or("no block tasks")?;
    assert_eq!(inner.len(), 2);
    assert_eq!(inner[0]["name"], "Stage /etc/app/tls.env");
    assert_eq!(
        inner[0]["changedWhen"],
        serde_json::json!({"kind": "literal", "value": false})
    );
    let dest = inner[0]["operation"]["dest"].as_str().ok_or("no dest")?;
    assert!(dest.ends_with(".ruxel-candidate"), "{dest}");
    // Root's alone, whatever the file's own owner and mode.
    for (field, value) in [("mode", "0600"), ("owner", "root"), ("group", "root")] {
        assert_eq!(inner[0]["operation"][field], value, "{field}");
    }
    assert_eq!(inner[1]["name"], "Validate /etc/app/tls.env");
    assert_eq!(inner[1]["operation"]["kind"], "command");
    let argv: Vec<&str> = inner[1]["operation"]["argv"]
        .as_array()
        .ok_or("no argv")?
        .iter()
        .map(|argument| argument.as_str().unwrap_or_default())
        .collect();
    assert_eq!(argv, ["app", "check", "--config", dest]);
    // The cleanup runs whether the validator succeeds or fails.
    let always = block["always"].as_array().ok_or("no always tasks")?;
    assert_eq!(always.len(), 1);
    assert_eq!(always[0]["operation"]["path"], dest);
    assert_eq!(always[0]["operation"]["state"], "absent");
    assert_eq!(
        always[0]["changedWhen"],
        serde_json::json!({"kind": "literal", "value": false})
    );
    // The publish task follows the block, outside it.
    let position = all
        .iter()
        .position(|task| task["name"] == "Validate /etc/app/tls.env")
        .ok_or("no block position")?;
    assert_eq!(all[position + 1]["name"], "Write /etc/app/tls.env");
    assert_eq!(all[position + 1]["operation"]["dest"], "/etc/app/tls.env");
    Ok(())
}

/// Regex match offsets count UTF-16 code units, while Pkl string indexes
/// count code points. A non-BMP character around a secret must not shift the
/// literal text on either side of the lookup.
#[test]
fn unicode_around_a_secret_keeps_the_literal_text_intact() -> Result {
    let config = valid("secret-file-unicode.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["operation"]["dest"] == "/etc/app/unicode.env")
        .ok_or("no unicode secret task")?;
    assert_eq!(
        task["operation"]["content"],
        serde_json::json!({
            "kind": "call",
            "function": "concat",
            "args": [
                {"kind": "literal", "value": "😀P="},
                {
                    "kind": "lookup",
                    "provider": "fnox",
                    "key": "AB",
                    "argv": [],
                },
                {"kind": "literal", "value": "TAIL😀"},
            ],
        })
    );
    Ok(())
}

/// A three-digit mode spells the same file mode as its four-digit form:
/// "600" is 0600, narrow enough for secret content.
#[test]
fn short_secret_file_modes_lower() -> Result {
    let config = valid("secret-file-short-mode.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["operation"]["dest"] == "/etc/lightdash/short.env")
        .ok_or("no short-mode task")?;
    assert_eq!(task["operation"]["mode"], "600");
    assert_eq!(task["noLog"], serde_json::json!(true));
    Ok(())
}

/// `SecretText.join` stays one secret: the pair lowers as two lookups with
/// the separator between them, in the file content's own concat.
#[test]
fn a_joined_secret_pair_lowers_as_two_lookups() -> Result {
    let config = valid("secret-join.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Write the bootstrap token for this node")
        .ok_or("no token file task")?;
    assert_eq!(task["noLog"], serde_json::json!(true));
    assert_eq!(task["operation"]["mode"], "0400");
    let args = task["operation"]["content"]["args"]
        .as_array()
        .ok_or("no concat args")?;
    let providers: Vec<&str> = args
        .iter()
        .filter(|arg| arg["kind"] == "lookup")
        .filter_map(|lookup| lookup["key"].as_str())
        .collect();
    assert_eq!(providers, ["KUBEADM_TOKEN_ID", "KUBEADM_TOKEN_SECRET"]);
    assert!(
        args.iter()
            .any(|arg| arg == &serde_json::json!({"kind": "literal", "value": "."}))
    );
    Ok(())
}

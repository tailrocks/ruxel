//! The secret-bearing workflows of the corpus in `tests/workflow`.
//!
//! A secret lowers to an engine lookup inside the workflow's own values,
//! and the task that carries it is never logged.

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

fn valid(name: &str) -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &root().join("valid").join(name),
        &options(),
    )?)
}

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
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
/// "600" is 0600, narrow enough for secret content. Lowering emits the
/// canonical four-digit spelling, so the engine reads one mode, not one
/// spelling.
#[test]
fn short_secret_file_modes_lower() -> Result {
    let config = valid("secret-file-short-mode.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["operation"]["dest"] == "/etc/lightdash/short.env")
        .ok_or("no short-mode task")?;
    assert_eq!(task["operation"]["mode"], "0600");
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

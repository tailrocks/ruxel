//! The apt completions of the workflow language: the assertions of the
//! corpus in `tests/workflow` that the apt preferences, the debconf
//! selections and the package holds need.
//!
//! The corpus harness itself lives in `workflow.rs`; this file holds the
//! assertions of the CAT.04 blocks, which the size policy keeps apart
//! from it.

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

/// One apt preference stanza is one managed change: the whole stanza is
/// written in one copy task under `/etc/apt/preferences.d/`, never a part
/// of it and never a second task for the same stanza. The selector
/// survives into the stanza, so an architecture and a glob are both
/// written as they are named (T15, T16, T19).
#[test]
fn apt_preference_writes_one_stanza_file() -> Result {
    let config = valid("apt-preference.pkl")?;
    let all = tasks(&config);
    let stanzas: Vec<&serde_json::Value> = all
        .iter()
        .copied()
        .filter(|task| {
            task["operation"]["dest"]
                .as_str()
                .is_some_and(|dest| dest.starts_with("/etc/apt/preferences.d/"))
        })
        .collect();
    // Two stanzas in the first stage (one of them listed twice), the
    // 32767 boundary stanza of the first stage, and one in the second:
    // four tasks, one per stanza.
    assert_eq!(stanzas.len(), 4, "{stanzas:#?}");
    for stanza in &stanzas {
        assert_eq!(stanza["operation"]["kind"], "copy");
        assert_eq!(stanza["operation"]["mode"], "0644");
        assert_eq!(stanza["operation"]["owner"], "root");
        assert_eq!(stanza["operation"]["group"], "root");
        let content = stanza["operation"]["content"]
            .as_str()
            .ok_or("no stanza content")?;
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "{content}");
        assert!(lines[0].starts_with("Package: "), "{content}");
        assert!(lines[1].starts_with("Pin: "), "{content}");
        assert!(lines[2].starts_with("Pin-Priority: "), "{content}");
    }
    // The selector is written as it is named: an architecture and a glob
    // are both part of the stanza.
    let contents: Vec<&str> = stanzas
        .iter()
        .filter_map(|stanza| stanza["operation"]["content"].as_str())
        .collect();
    assert!(
        contents
            .iter()
            .any(|content| content.contains("Package: nginx:amd64\n")),
        "{contents:?}"
    );
    assert!(
        contents
            .iter()
            .any(|content| content.contains("Package: docker*\n")),
        "{contents:?}"
    );
    Ok(())
}

/// The identity of a preference is the package plus the pin: the same
/// stanza listed twice is one step, and the same package with another pin
/// is another step, which watches its own result (T17).
#[test]
fn apt_preference_identity_is_the_package_and_the_pin() -> Result {
    let config = valid("apt-preference.pkl")?;
    let all = tasks(&config);
    let stanza = |pin: &str| -> Result<&serde_json::Value> {
        all.iter()
            .copied()
            .find(|task| {
                task["operation"]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(&format!("Pin: {pin}\n")))
            })
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("no stanza with pin {pin}").into()
            })
    };
    let first = stanza("version 1.25.*")?;
    let second = stanza("release n=stable")?;
    // Another pin for the same package is another step.
    let first_register = first["register"].as_str().ok_or("no register")?;
    let second_register = second["register"].as_str().ok_or("no register")?;
    assert_ne!(first_register, second_register, "two pins, two steps");
    // The same stanza listed twice in the first stage is one step: one
    // task, not two.
    let plays = config["plays"].as_array().ok_or("no plays")?;
    let first_stage = plays[0]["tasks"].as_array().ok_or("no tasks")?;
    let repeated = first_stage
        .iter()
        .filter(|task| task["operation"]["dest"] == first["operation"]["dest"])
        .count();
    assert_eq!(repeated, 1, "the same stanza twice is one step");
    // Each watcher reacts to the step it names.
    let watcher = |name: &str| -> Result<Vec<String>> {
        let task = all
            .iter()
            .copied()
            .find(|task| task["name"] == name)
            .ok_or_else(|| -> Box<dyn std::error::Error> { format!("no task {name:?}").into() })?;
        Ok(watched_results(&task["when"]))
    };
    assert_eq!(watcher("Reload nginx")?, [first_register.to_owned()]);
    assert_eq!(watcher("Reload nginx again")?, [second_register.to_owned()]);
    Ok(())
}

/// A debconf selection lowers to one task that carries the package, the
/// question, the type and the answer as they are written (T23).
#[test]
fn debconf_entry_lowers_to_a_selections_task() -> Result {
    let config = valid("debconf-entry.pkl")?;
    let all = tasks(&config);
    let selections: Vec<&serde_json::Value> = all
        .iter()
        .copied()
        .filter(|task| task["operation"]["kind"] == "debconf")
        .collect();
    assert_eq!(selections.len(), 2, "{selections:#?}");
    let static_answer = selections
        .iter()
        .copied()
        .find(|task| task["operation"]["value"]["kind"] == "literal")
        .ok_or("no static answer")?;
    assert_eq!(static_answer["operation"]["package"], "postfix");
    assert_eq!(
        static_answer["operation"]["question"],
        "postfix/main_mailer_type"
    );
    assert_eq!(static_answer["operation"]["type"], "select");
    assert_eq!(
        static_answer["operation"]["value"],
        serde_json::json!({"kind": "literal", "value": "Local only"})
    );
    // A static answer is nothing to hide: the task is logged.
    assert_eq!(static_answer["noLog"], serde_json::json!(false));
    Ok(())
}

/// A secret answer lowers to a lookup the engine resolves in its store,
/// and the task that carries it is never logged (T24).
#[test]
fn a_debconf_secret_answer_is_a_lookup_and_is_never_logged() -> Result {
    let config = valid("debconf-entry.pkl")?;
    let all = tasks(&config);
    let secret = all
        .iter()
        .copied()
        .find(|task| {
            task["operation"]["kind"] == "debconf" && task["operation"]["value"]["kind"] == "lookup"
        })
        .ok_or("no secret answer")?;
    assert_eq!(secret["operation"]["package"], "postfix");
    assert_eq!(secret["operation"]["question"], "postfix/relay_pw");
    assert_eq!(secret["operation"]["type"], "password");
    assert_eq!(
        secret["operation"]["value"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "fnox",
            "key": "POSTFIX_RELAY_PW",
            "argv": [],
        })
    );
    assert_eq!(secret["noLog"], serde_json::json!(true));
    Ok(())
}

/// A hold is part of the identity of a package step: two steps that differ
/// only in the hold are two steps, and each watcher reacts to its own
/// result (T28).
#[test]
fn a_hold_is_part_of_the_step_identity() -> Result {
    let config = valid("apt-hold.pkl")?;
    let all = tasks(&config);
    let watcher = |name: &str| -> Result<Vec<String>> {
        let task = all
            .iter()
            .copied()
            .find(|task| task["name"] == name)
            .ok_or_else(|| -> Box<dyn std::error::Error> { format!("no task {name:?}").into() })?;
        Ok(watched_results(&task["when"]))
    };
    let plain = watcher("Reload plain")?;
    let held = watcher("Reload held")?;
    assert_eq!(plain.len(), 1, "{plain:?}");
    assert_eq!(held.len(), 1, "{held:?}");
    assert_ne!(plain, held, "a hold is part of the identity");
    // Each watcher reacts to the step it names, and the two steps are two
    // tasks.
    let registered = |id: &str| -> Result<&serde_json::Value> {
        all.iter()
            .copied()
            .find(|task| task["register"] == id)
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("no step registered {id:?}").into()
            })
    };
    assert_eq!(
        registered(&plain[0])?["operation"]["hold"],
        serde_json::Value::Null
    );
    assert_eq!(
        registered(&held[0])?["operation"]["hold"],
        serde_json::json!(true)
    );
    Ok(())
}

/// A held package is described as held: the plan line of a step says what
/// it does, so a held step and a plain step of the same package are told
/// apart (T29).
#[test]
fn a_held_package_is_described_as_held() -> Result {
    let config = valid("apt-hold.pkl")?;
    let all = tasks(&config);
    let name = |wanted: &str| -> Result<String> {
        all.iter()
            .copied()
            .find(|task| task["name"] == wanted)
            .map(|task| task["name"].as_str().unwrap_or_default().to_owned())
            .ok_or_else(|| -> Box<dyn std::error::Error> { format!("no task {wanted:?}").into() })
    };
    assert_eq!(name("Install nginx")?, "Install nginx");
    assert_eq!(name("Install and hold nginx")?, "Install and hold nginx");
    // A step with a title keeps it: the title is the plan line.
    assert_eq!(name("Install curl")?, "Install curl");
    Ok(())
}

/// The name of a preference fragment is one apt reads: `apt_preferences(5)`
/// loads only fragments without an extension or with the `.pref`
/// extension, and their names carry only alphanumeric characters, dashes,
/// underscores and periods, so every generated fragment is one apt loads
/// (T37).
#[test]
fn apt_preference_file_names_are_apt_compatible() -> Result {
    let config = valid("apt-preference.pkl")?;
    let all = tasks(&config);
    let dests: Vec<&str> = all
        .iter()
        .filter_map(|task| task["operation"]["dest"].as_str())
        .filter(|dest| dest.starts_with("/etc/apt/preferences.d/"))
        .collect();
    assert_ne!(dests.len(), 0, "no apt preference fragment was planned");
    for dest in &dests {
        let name = dest.rsplit('/').next().unwrap_or_default();
        assert!(
            Path::new(name).extension().is_some_and(|ext| ext == "pref"),
            "{name}"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "{name}"
        );
    }
    Ok(())
}

/// Two stanzas never share a fragment file: the fragment name carries the
/// digest of the selector, so two selectors that sanitize to one basename
/// still write one file each (review thread `PRRT_kwDOS3Fimc62bqId`). The
/// pair below once shared both the sanitized basename and an
/// eight-hex-digit digest prefix (`7df67fd2`).
#[test]
fn apt_preference_file_names_do_not_collide() -> Result {
    let config = valid("apt-preference-collision.pkl")?;
    let all = tasks(&config);
    let dests: Vec<&str> = all
        .iter()
        .filter_map(|task| task["operation"]["dest"].as_str())
        .filter(|dest| dest.starts_with("/etc/apt/preferences.d/"))
        .collect();
    assert_eq!(dests.len(), 2, "{dests:?}");
    let mut distinct: Vec<&str> = dests.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        2,
        "the two stanzas must not share a fragment file: {dests:?}"
    );
    Ok(())
}

/// A long selector still writes a legal fragment file: the fragment name
/// carries the sanitized selector and the digest of the selector, and
/// Linux cannot create a file whose name exceeds `NAME_MAX` (255 bytes).
/// The selector here is 186 characters, which makes a 256-byte name when
/// the selector part is written whole, so the name caps the selector
/// part and keeps the full digest (T38).
#[test]
fn apt_preference_file_names_fit_name_max() -> Result {
    let config = valid("apt-preference-long-selector.pkl")?;
    let all = tasks(&config);
    let dests: Vec<&str> = all
        .iter()
        .filter_map(|task| task["operation"]["dest"].as_str())
        .filter(|dest| dest.starts_with("/etc/apt/preferences.d/"))
        .collect();
    assert_eq!(dests.len(), 1, "{dests:?}");
    for dest in &dests {
        let name = dest.rsplit('/').next().unwrap_or_default();
        assert!(
            name.len() <= 255,
            "the fragment name exceeds NAME_MAX (255 bytes): {name} ({} bytes)",
            name.len()
        );
        assert!(
            Path::new(name).extension().is_some_and(|ext| ext == "pref"),
            "{name}"
        );
    }
    Ok(())
}

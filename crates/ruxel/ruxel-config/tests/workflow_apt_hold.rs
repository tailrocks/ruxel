//! The plan-level face of a pinned hold (CAT.04, Ym8H): the pin of a
//! `hold` step survives into the engine's apt operation, so the agent
//! installs exactly the version the step names and then freezes it.
//!
//! The corpus assertions of the other apt completions live in
//! `workflow_apt.rs`; this file is apart from it for the same size policy.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

fn valid(name: &str) -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &root().join("valid").join(name),
        &ruxel_config::EvalOptions {
            root_dir: Some(root()),
            ..ruxel_config::EvalOptions::default()
        },
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

/// A hold with pinned names lowers to one apt operation that carries the
/// pins as they are written and the hold on: the agent installs the named
/// versions and holds them, even when the host already holds another
/// version.
#[test]
fn a_pinned_hold_lowers_with_its_pins_and_the_hold() -> Result {
    let config = valid("apt-hold.pkl")?;
    let pinned = |task: &&serde_json::Value| {
        task["operation"]["kind"] == "apt"
            && task["operation"]["hold"] == true
            && task["operation"]["name"].as_array().is_some_and(|names| {
                names
                    .iter()
                    .any(|name| name.as_str().is_some_and(|name| name.contains('=')))
            })
    };
    let task = tasks(&config)
        .into_iter()
        .find(pinned)
        .ok_or("no pinned hold task")?;
    let name = &task["operation"]["name"];
    let names: Vec<&str> = name
        .as_array()
        .ok_or("a pinned hold takes names")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(
        names.contains(&"kubelet=1.37.2-1.1") && names.contains(&"kubeadm=1.37.2-1.1"),
        "{names:?}"
    );
    assert_eq!(task["operation"]["state"], "present");
    assert_eq!(task["operation"]["hold"], true);
    Ok(())
}

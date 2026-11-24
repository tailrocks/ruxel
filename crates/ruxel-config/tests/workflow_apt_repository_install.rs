//! Installing from a workflow-written repository: the corpus assertions
//! of the repository, refresh, install shape. The apt completions of the
//! workflow language live in `workflow_apt.rs`; this file is apart from
//! it for the same size policy.
//!

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/workflow")).to_path_buf()
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

/// An install ordered after a refresh that is ordered after a
/// workflow-written repository plans against the published candidates:
/// the repository, the refresh and the install lower to tasks in the
/// author's order, and the install stays a plain ordered step — no
/// second shape, no watcher edge. The PGDG suite is static text the way
/// the postgresql-nova play writes it.
#[test]
fn an_install_after_a_refreshed_repository_lowers_in_order() -> Result {
    let config = valid("install-base-repository-install.pkl")?;
    let all = tasks(&config);
    let position = |name: &str| -> Result<usize> {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| -> Box<dyn std::error::Error> { format!("no task {name:?}").into() })
    };
    let keyrings = position("Create /etc/apt/keyrings")?;
    let key = position("Install pgdg apt signing key")?;
    let sources = position("Add the PostgreSQL repository")?;
    let refresh = position("Refresh the index for pgdg")?;
    let install = position("Install PostgreSQL 19")?;
    assert!(
        keyrings < key && key < sources && sources < refresh && refresh < install,
        "the repository, the refresh and the install must lower in the author's order"
    );
    let sources_task = &all[sources];
    assert_eq!(
        sources_task["operation"]["dest"],
        "/etc/apt/sources.list.d/pgdg.sources"
    );
    let content = sources_task["operation"]["content"]
        .as_str()
        .ok_or("no sources content")?;
    assert!(content.contains("Suites: trixie-pgdg\n"), "{content}");
    assert!(
        content.contains("Signed-By: /etc/apt/keyrings/pgdg.asc\n"),
        "{content}"
    );
    let install_task = &all[install];
    assert_eq!(install_task["operation"]["kind"], "apt");
    let names = install_task["operation"]["name"].clone();
    assert_eq!(
        names,
        serde_json::json!(["postgresql-19", "postgresql-server-dev-19"]),
        "{names}"
    );
    Ok(())
}

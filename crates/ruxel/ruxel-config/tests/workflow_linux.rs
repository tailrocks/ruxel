//! The Linux catalogue's identity rules, on the corpus: an account's
//! supplementary groups are a set, so order and repeats are one state.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

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

/// The groups of an account are a set: the engine reconciles supplementary
/// groups order-insensitively (it sorts and deduplicates what it passes to
/// usermod), so the same members in another order — or spelled with a
/// repeat — are one identity, and the three steps of the corpus case lower
/// to one task, not a "different settings" conflict.
#[test]
fn account_groups_are_a_set_not_an_order() -> Result {
    let config = valid("linux-user-groups-order.pkl")?;
    let accounts: Vec<&serde_json::Value> = tasks(&config)
        .into_iter()
        .filter(|task| task["operation"]["kind"] == "user")
        .collect();
    assert_eq!(accounts.len(), 1, "one identity: {accounts:#?}");
    assert_eq!(accounts[0]["name"], "Manage account app");
    assert_eq!(
        accounts[0]["operation"]["groups"],
        serde_json::json!(["docker", "wheel"])
    );
    Ok(())
}

//! The Linux catalogue's identity rules and lowering, on the corpus: an
//! account's supplementary groups are a set, so order and repeats are one
//! state; a sysctl reconciles its live kernel value on every apply; and
//! the account step lowers under the `user` kind, not the `loginShell` one
//! the migration replaced.

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
    // The retained occurrence carries a repeat; the operation carries the
    // canonical set, or the agent compares against its own deduplicated
    // list and reports perpetual change.
    assert_eq!(
        accounts[0]["operation"]["groups"],
        serde_json::json!(["docker", "wheel"])
    );
    Ok(())
}

/// An explicit empty supplementary-group list is a state: the operation
/// carries `groups = []`, so the agent removes every membership. A step
/// that states no `groups` emits no field at all and leaves memberships
/// unmanaged.
#[test]
fn explicit_empty_groups_are_emitted_and_unset_ones_are_not() -> Result {
    let config = valid("linux-user-groups-empty.pkl")?;
    let account = |user: &str| -> Result<serde_json::Value> {
        Ok(tasks(&config)
            .into_iter()
            .find(|task| task["operation"]["kind"] == "user" && task["operation"]["name"] == user)
            .ok_or_else(|| format!("no task for {user}"))?
            .clone())
    };
    assert_eq!(
        account("app-empty")?["operation"]["groups"],
        serde_json::json!([]),
        "an explicit empty list must reconcile to none"
    );
    assert!(
        account("app-free")?["operation"].get("groups").is_none()
            || account("app-free")?["operation"]["groups"].is_null(),
        "an unmanaged list must emit no groups field"
    );
    Ok(())
}

/// A sysctl step must reconcile the live kernel value on every apply: the
/// lowered operation sets `sysctl_set`, whose engine default is false —
/// without it a persisted entry is never read back against the kernel, so
/// a parameter another process changed is never corrected.
#[test]
fn sysctl_reconciles_the_live_kernel_value() -> Result {
    let config = valid("linux-sysctl.pkl")?;
    let sysctls: Vec<&serde_json::Value> = tasks(&config)
        .into_iter()
        .filter(|task| task["operation"]["kind"] == "sysctl")
        .collect();
    assert_eq!(sysctls.len(), 2, "two parameters: {sysctls:#?}");
    for sysctl in sysctls {
        assert_eq!(
            sysctl["operation"]["sysctl_set"],
            serde_json::json!(true),
            "live-value reconciliation must be on: {sysctl:#}"
        );
    }
    Ok(())
}

/// The account step lowers under the `user` kind: a dependent's reference
/// and the step's own register name are spelled `user_<hex>`, so the
/// `loginShell` kind of the old login-shell model leaks into no generated
/// identity or result name.
#[test]
fn the_account_step_kind_is_user() -> Result {
    let config = valid("linux-user-watch.pkl")?;
    let all = tasks(&config);
    let command = all
        .iter()
        .find(|task| task["name"] == "Verify the account")
        .ok_or("no dependent command")?;
    let watched = command["when"]["path"][1]
        .as_str()
        .ok_or("the command watches no result")?;
    assert!(
        watched.starts_with("user_"),
        "the reference must carry the user kind: {watched}"
    );
    let account = all
        .iter()
        .find(|task| task["name"] == "Manage account ruxel-app")
        .ok_or("no account task")?;
    assert_eq!(account["register"], serde_json::json!(watched));
    Ok(())
}

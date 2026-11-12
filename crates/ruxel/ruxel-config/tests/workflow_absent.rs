//! The absent forms: an `Absent` step and an iptables rule lower to the
//! engine's operations.

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

/// An `Absent` step lowers to the engine's file operation with the state
/// "absent" and the recursion as the author wrote it.
#[test]
fn absent_lowers_to_file_absent() -> Result {
    let config = valid("absent.pkl")?;
    let all = tasks(&config);
    let named = |name: &str| {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| format!("no task {name:?}"))
    };
    let file = named("Remove /etc/obsolete.conf")?;
    assert_eq!(all[file]["operation"]["kind"], "file");
    assert_eq!(all[file]["operation"]["state"], "absent");
    assert_eq!(all[file]["operation"]["path"], "/etc/obsolete.conf");
    assert_eq!(all[file]["operation"]["recurse"], false);
    let directory = named("Remove /var/tmp/old-cache")?;
    assert_eq!(all[directory]["operation"]["kind"], "file");
    assert_eq!(all[directory]["operation"]["state"], "absent");
    assert_eq!(all[directory]["operation"]["path"], "/var/tmp/old-cache");
    assert_eq!(all[directory]["operation"]["recurse"], true);
    Ok(())
}

/// An iptables rule lowers to the engine's firewall operation, with the
/// field names the engine uses.
#[test]
fn iptables_rule_lowers_snake_case() -> Result {
    let config = valid("iptables-rule.pkl")?;
    let all = tasks(&config);
    let named = |name: &str| {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| format!("no task {name:?}"))
    };
    let rule = named("Add iptables rule to INPUT")?;
    assert_eq!(all[rule]["operation"]["kind"], "iptables");
    assert_eq!(all[rule]["operation"]["chain"], "INPUT");
    assert_eq!(all[rule]["operation"]["protocol"], "tcp");
    assert_eq!(all[rule]["operation"]["destination"], "10.0.0.0/8");
    assert_eq!(all[rule]["operation"]["jump"], "DROP");
    assert_eq!(all[rule]["operation"]["ip_version"], "ipv4");
    assert_eq!(all[rule]["operation"]["state"], "present");
    let second = named("Add iptables rule to OUTPUT")?;
    assert_eq!(all[second]["operation"]["out_interface"], "eth0");
    assert_eq!(all[second]["operation"]["ip_version"], "ipv6");
    // The jump defaults to DROP.
    assert_eq!(all[second]["operation"]["jump"], "DROP");
    Ok(())
}

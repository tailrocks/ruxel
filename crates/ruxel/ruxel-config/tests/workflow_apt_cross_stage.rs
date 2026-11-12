//! The cross-stage planning refusals and their host gate: a preference
//! or an index refresh in an earlier stage diverges from a later plan on
//! the same host — check and diff skip the write, apply performs it —
//! while stages on other hosts share no apt policy and are admitted.
//!
//! The corpus assertions of the same-stage rules live in `workflow_apt.rs`;
//! this file is apart from it for the same size policy.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

fn eval(name: &str) -> Result<serde_json::Value> {
    Ok(ruxel_config::eval(
        &root().join(name),
        &ruxel_config::EvalOptions {
            root_dir: Some(root()),
            ..ruxel_config::EvalOptions::default()
        },
    )?)
}

#[test]
fn a_preference_in_an_earlier_stage_of_the_same_host_is_refused() -> Result {
    let Err(error) = eval("invalid/apt-preference-before-latest-later-stage.pkl") else {
        return Err("the earlier preference is admitted".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("plans against the candidate policy")
            && message.contains("put the two stages on different hosts"),
        "{message}"
    );
    Ok(())
}

#[test]
fn an_index_in_an_earlier_stage_of_the_same_host_is_refused() -> Result {
    let Err(error) = eval("invalid/apt-index-before-system-upgrade-later-stage.pkl") else {
        return Err("the earlier refresh is admitted".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("refreshes the package index")
            && message.contains("put the two stages on different hosts"),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_preference_and_a_package_step_on_different_hosts_are_admitted() -> Result {
    eval("valid/apt-preference-different-hosts.pkl")?;
    Ok(())
}

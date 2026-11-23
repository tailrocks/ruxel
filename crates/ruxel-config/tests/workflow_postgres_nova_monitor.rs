//! The nova restart report and monitor wiring: the post-reload stage,
//! the daemon reload of the monitor service, the repeating timer, and the
//! hardened monitor script.
//!
//! The reload, leak and workload assertions live in
//! `workflow_postgres_nova.rs`, which the size policy keeps apart from
//! this file.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn workflows() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows")).to_path_buf()
}

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
}

fn workload() -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &workflows().join("setup-postgresql-nova.pkl"),
        &ruxel_config::EvalOptions {
            root_dir: Some(workflows()),
            ..ruxel_config::EvalOptions::default()
        },
    )?)
}

/// Handlers flush when the play ends, so the report reads the settings
/// after the reload ran: the report lives in its own stage, not beside
/// the service.
#[test]
fn nova_restart_report_runs_after_the_reload() -> Result {
    let config = workload()?;
    let plays = config["plays"].as_array().ok_or("no plays")?;
    assert_eq!(
        plays[2]["name"],
        "Report the settings a reload could not adopt"
    );
    let report = plays[2]["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["name"] == "Report settings that still require a restart")
        .ok_or("no restart report")?;
    assert_eq!(report["checkMode"], serde_json::json!(false));
    assert_eq!(report["becomeUser"], "postgres");
    assert!(
        plays[2]["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|task| task["name"]
                == "Warn when a restart is needed to finish applying the config")
    );
    assert!(
        !plays[1]["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|task| task["name"] == "Report settings that still require a restart"),
        "the report reads post-reload settings, so it runs after the handlers flush"
    );
    Ok(())
}

/// The monitor's service file reloads systemd on change, the way the
/// playbook's handler does when either unit file changed; the timer fires
/// past boot and every five minutes after, starting now; and the hardened
/// monitor pins its search path before querying.
#[test]
fn nova_monitor_reload_timer_and_hardening() -> Result {
    let config = workload()?;
    let all = tasks(&config);
    let reload = all
        .iter()
        .find(|task| task["name"] == "Reload systemd")
        .ok_or("no daemon reload")?;
    assert!(
        reload["operation"]["argv"]
            .as_array()
            .is_some_and(|argv| argv.iter().any(|argument| argument == "daemon-reload")),
        "{}",
        reload["operation"]
    );
    let timer_file = all
        .iter()
        .find(|task| {
            task["operation"]["dest"]
                == "/etc/systemd/system/chainargos-postgresql-multixact-monitor.timer"
        })
        .ok_or("no monitor timer")?;
    assert!(
        timer_file["operation"]["content"]
            .as_str()
            .is_some_and(|content| content.contains("OnUnitActiveSec=300")),
        "{}",
        timer_file["operation"]["content"]
    );
    let timer_enable = all
        .iter()
        .find(|task| task["name"] == "Enable timer chainargos-postgresql-multixact-monitor")
        .ok_or("no timer enablement")?;
    assert_eq!(timer_enable["operation"]["state"], "started");
    let monitor = all
        .iter()
        .find(|task| {
            task["operation"]["dest"] == "/usr/local/sbin/chainargos-postgresql-multixact-monitor"
        })
        .ok_or("no monitor script")?;
    assert!(
        monitor["operation"]["content"]
            .to_string()
            .contains("SET search_path TO pg_catalog"),
        "{}",
        monitor["operation"]["content"]
    );
    Ok(())
}

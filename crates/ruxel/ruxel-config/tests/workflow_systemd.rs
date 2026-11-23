//! The systemd blocks of the corpus in `tests/workflow`: a service that
//! takes reload triggers, and a systemd timer.
//!
//! A reload is a handler kind of its own, beside the restart, and a timer
//! lowers to its own unit file and to the enablement of the `.timer` unit.
//!
//! The delay cases (whole-millisecond precision and limits) live in
//! `workflow_systemd_timer_delay.rs`.

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

/// A reload-watching service lowers to a handler of its own.
#[test]
fn a_reload_service_lowers_to_a_reload_handler() -> Result {
    let config = valid("service-reload.pkl")?;
    let play = &config["plays"][0];
    let handlers: Vec<&serde_json::Value> =
        play["handlers"].as_array().into_iter().flatten().collect();
    let handler = |name: &str| {
        handlers
            .iter()
            .find(|handler| handler["name"] == name)
            .ok_or_else(|| format!("no handler {name:?}"))
    };
    let reload = handler("Reload postgresql")?;
    assert_eq!(reload["operation"]["kind"], "systemd");
    assert_eq!(reload["operation"]["state"], "reloaded");
    assert_eq!(reload["operation"]["name"], "postgresql");
    let restart = handler("Restart nginx")?;
    assert_eq!(restart["operation"]["state"], "restarted");
    let mut names: Vec<&str> = handlers
        .iter()
        .map(|handler| handler["name"].as_str().unwrap_or_default())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["Reload postgresql", "Restart nginx"]);
    let all = tasks(&config);
    let config_file = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/etc/postgresql/18/main/postgresql.conf")
        .ok_or("no configuration task")?;
    assert_eq!(
        config_file["notify"],
        serde_json::json!(["Reload postgresql"])
    );
    let nginx_config = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/etc/nginx/nginx.conf")
        .ok_or("no nginx configuration task")?;
    assert_eq!(nginx_config["notify"], serde_json::json!(["Restart nginx"]));
    Ok(())
}

/// A timer lowers to its unit file and to its enablement.
#[test]
fn a_timer_lowers_to_its_unit_file_and_its_enablement() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let timer_file = all
        .iter()
        .find(|task| {
            task["operation"]["dest"] == "/etc/systemd/system/postgresql-multixact-monitor.timer"
        })
        .ok_or("no timer file task")?;
    let content = timer_file["operation"]["content"]
        .as_str()
        .ok_or("no timer file content")?;
    assert!(content.contains("OnCalendar=daily"), "{content}");
    assert!(content.contains("OnBootSec=120"), "{content}");
    assert!(content.contains("OnUnitActiveSec=300"), "{content}");
    assert!(content.contains("Persistent=true"), "{content}");
    assert!(content.contains("WantedBy=timers.target"), "{content}");
    let enable = all
        .iter()
        .find(|task| task["name"] == "Enable timer postgresql-multixact-monitor")
        .ok_or("no enablement task")?;
    assert_eq!(enable["operation"]["kind"], "systemd");
    assert_eq!(
        enable["operation"]["name"],
        "postgresql-multixact-monitor.timer"
    );
    assert_eq!(enable["operation"]["enabled"], true);
    let disable = all
        .iter()
        .find(|task| task["name"] == "Disable timer postgresql-backup")
        .ok_or("no disablement task")?;
    assert_eq!(disable["operation"]["name"], "postgresql-backup.timer");
    assert_eq!(disable["operation"]["enabled"], false);
    Ok(())
}

/// A timer names the service it starts in the unit file.
#[test]
fn a_timer_names_the_service_it_starts() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let timer_file = all
        .iter()
        .find(|task| {
            task["operation"]["dest"] == "/etc/systemd/system/postgresql-multixact-monitor.timer"
        })
        .ok_or("no timer file task")?;
    let content = timer_file["operation"]["content"]
        .as_str()
        .ok_or("no timer file content")?;
    assert!(
        content.contains("Unit=postgresql-multixact-monitor.service"),
        "{content}"
    );
    Ok(())
}

/// A timer without a service starts the service of its own name.
#[test]
fn a_timer_without_a_service_starts_the_service_of_its_own_name() -> Result {
    let config = valid("systemd-timer-default-service.pkl")?;
    let all = tasks(&config);
    let timer_file = all
        .iter()
        .find(|task| {
            task["operation"]["dest"] == "/etc/systemd/system/postgresql-maintenance.timer"
        })
        .ok_or("no timer file task")?;
    let content = timer_file["operation"]["content"]
        .as_str()
        .ok_or("no timer file content")?;
    assert!(
        content.contains("Unit=postgresql-maintenance.service"),
        "{content}"
    );
    Ok(())
}

/// A repeat lowers to `OnUnitActiveSec`, beside the boot delay.
#[test]
fn a_repeat_lowers_to_on_unit_active_sec() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let content = |unit: &str| {
        all.iter()
            .find(|task| task["operation"]["dest"] == unit)
            .and_then(|task| task["operation"]["content"].as_str().map(str::to_owned))
            .ok_or_else(|| format!("no timer file {unit}"))
    };
    let repeating = content("/etc/systemd/system/postgresql-multixact-monitor.timer")?;
    assert!(repeating.contains("OnUnitActiveSec=300"), "{repeating}");
    // A timer without a repeat carries no interval line.
    let quick = content("/etc/systemd/system/postgresql-quick-check.timer")?;
    assert!(!quick.contains("OnUnitActiveSec"), "{quick}");
    Ok(())
}

/// The monitor workload is the whole composition: the executable
/// script, the service unit, the timer, and the daemon-reload ordered
/// after the unit file and before the timer.
#[test]
fn a_timer_workload_orders_script_service_reload_and_timer() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let position = |name: &str| {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| format!("no task {name:?}"))
    };
    let script = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/usr/local/bin/multixact-monitor")
        .ok_or("no monitor script task")?;
    assert_eq!(script["operation"]["mode"], "0755");
    let script_at = position("Write /usr/local/bin/multixact-monitor")?;
    let unit_at = position("Write /etc/systemd/system/postgresql-multixact-monitor.service")?;
    let reload_at = position("Reload systemd")?;
    let timer_at = position("Manage timer postgresql-multixact-monitor on daily")?;
    let enable_at = position("Enable timer postgresql-multixact-monitor")?;
    assert!(
        script_at < unit_at && unit_at < reload_at && reload_at < timer_at && timer_at < enable_at,
        "script, unit, reload, timer and enablement run in order"
    );
    Ok(())
}

/// A timer owns its enablement link.
#[test]
fn a_timer_refuses_a_second_state_for_its_enablement_link() -> Result {
    let path = root()
        .join("invalid")
        .join("timer-enablement-symlink-conflict.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "both manage /etc/systemd/system/timers.target.wants/app.timer: \
                    a SystemdTimer and a Symlink; a stage declares one state for each \
                    thing, so put the second state in a later stage";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// The timer's result carries the change of both of its tasks.
#[test]
fn a_timer_reports_the_change_of_its_file_and_its_enablement() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let file = all
        .iter()
        .find(|task| {
            task["operation"]["dest"] == "/etc/systemd/system/postgresql-multixact-monitor.timer"
        })
        .ok_or("no timer file task")?;
    let enable = all
        .iter()
        .find(|task| task["name"] == "Enable timer postgresql-multixact-monitor")
        .ok_or("no enablement task")?;
    // The enablement task holds the step's result, and folds the change of
    // the unit file into it: the unit file's own result is registered
    // beside it.
    let register = enable["register"]
        .as_str()
        .ok_or("the enablement task registers no result")?;
    assert_eq!(
        file["register"],
        serde_json::json!(format!("{register}_unit"))
    );
    assert_eq!(
        enable["changedWhen"],
        serde_json::json!({
            "kind": "call",
            "function": "or",
            "args": [
                {"kind": "ref", "path": ["result", "changed"]},
                {"kind": "ref", "path": ["results", format!("{register}_unit"), "changed"]}
            ]
        })
    );
    // The reload the rewritten file needs: asked whenever the file
    // changed, and ordered after the file write.
    assert_eq!(
        enable["operation"]["daemon_reload"],
        serde_json::json!({"kind": "ref", "path": ["results", format!("{register}_unit"), "changed"]})
    );
    assert!(
        all.iter().position(|t| *t == *file) < all.iter().position(|t| *t == *enable),
        "the reload must run after the file write"
    );
    Ok(())
}

/// A timer basename may be 249 characters beside the suffix.
#[test]
fn a_timer_basename_may_be_249_characters() -> Result {
    let config = valid("systemd-timer-long-name.pkl")?;
    let all = tasks(&config);
    let basename: String = "t".to_owned() + &"a".repeat(248);
    assert_eq!(basename.len(), 249);
    let dest = format!("/etc/systemd/system/{basename}.timer");
    let timer = all
        .iter()
        .find(|task| task["operation"]["dest"] == dest)
        .ok_or("no timer file")?;
    assert_eq!(timer["operation"]["dest"], dest);
    assert_eq!(basename.len() + ".timer".len(), 255);
    // The same basename naming no service is refused.
    let implicit = root()
        .join("invalid")
        .join("timer-long-name-implicit-service.pkl");
    let error = ruxel_config::eval_config(&implicit, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    assert!(error.contains("at most 247 characters"), "{error}");
    Ok(())
}

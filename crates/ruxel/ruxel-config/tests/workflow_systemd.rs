//! The systemd blocks of the corpus in `tests/workflow`: a service that
//! takes reload triggers, and a systemd timer.
//!
//! A reload is a handler kind of its own, beside the restart, and a timer
//! lowers to its own unit file and to the enablement of the `.timer` unit.

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
    // The restart of another service keeps its own handler kind.
    let restart = handler("Restart nginx")?;
    assert_eq!(restart["operation"]["state"], "restarted");
    // A reload-only service grows no restart handler: the two handler
    // kinds are not merged, and the play carries one handler per service
    // that takes triggers.
    let mut names: Vec<&str> = handlers
        .iter()
        .map(|handler| handler["name"].as_str().unwrap_or_default())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["Reload postgresql", "Restart nginx"]);
    // The watched step notifies the handler of its own service, and no
    // other.
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
    assert!(content.contains("OnBootSec=300"), "{content}");
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
    // A timer that is not enabled is disabled, and its unit file says so
    // by not carrying the enablement.
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

/// A fractional millisecond is refused, not rounded.
#[test]
fn a_fractional_boot_delay_is_refused() -> Result {
    for (name, delay) in [
        ("timer-boot-fractional-millisecond.pkl", "1000.0000005.ms"),
        ("timer-boot-half-nanosecond.pkl", "0.5.ns"),
        (
            "timer-boot-fractional-millisecond-large.pkl",
            "1.0000000000000005E9.ms",
        ),
        (
            "timer-boot-fractional-millisecond-huge.pkl",
            "2.0000000000000005E12.s",
        ),
        (
            "timer-boot-fractional-huge-integer.pkl",
            "9007199254740992.ns",
        ),
    ] {
        let path = root().join("invalid").join(name);
        let error = ruxel_config::eval_config(&path, &options())
            .err()
            .ok_or_else(|| format!("{name}: evaluated"))?
            .to_string();
        let expected = format!(
            "onBoot {delay} must be a whole number of milliseconds; \
             a systemd timer reads the delay in seconds or milliseconds"
        );
        assert!(error.contains(&expected), "{name}: {error}");
    }
    Ok(())
}

/// A delay keeps its precision: half a second is not zero.
#[test]
fn a_timer_keeps_the_precision_of_its_boot_delay() -> Result {
    let config = valid("systemd-timer.pkl")?;
    let all = tasks(&config);
    let content = |unit: &str| {
        all.iter()
            .find(|task| task["operation"]["dest"] == unit)
            .and_then(|task| task["operation"]["content"].as_str().map(str::to_owned))
            .ok_or_else(|| format!("no timer file {unit}"))
    };
    // Whole seconds stay seconds.
    let daily = content("/etc/systemd/system/postgresql-multixact-monitor.timer")?;
    assert!(daily.contains("OnBootSec=300"), "{daily}");
    // Milliseconds stay milliseconds.
    let quick = content("/etc/systemd/system/postgresql-quick-check.timer")?;
    assert!(quick.contains("OnBootSec=500ms"), "{quick}");
    assert!(!quick.contains("OnBootSec=0"), "{quick}");
    Ok(())
}

/// A large delay is kept as written; one past the exact limit is refused.
#[test]
fn a_large_boot_delay_keeps_its_milliseconds() -> Result {
    let config = valid("systemd-timer-large-delay.pkl")?;
    let all = tasks(&config);
    let content = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/etc/systemd/system/postgresql-long-delay.timer")
        .and_then(|task| task["operation"]["content"].as_str().map(str::to_owned))
        .ok_or("no timer file task")?;
    // A million seconds is a whole number of seconds, so systemd reads it
    // in seconds; the delay keeps every millisecond it was written with.
    assert!(content.contains("OnBootSec=1000000"), "{content}");
    // Every delay at or beyond the exact limit is refused with one sentence.
    for name in [
        "beyond-exact-limit",
        "beyond-exact-limit-seconds",
        "beyond-exact-limit-huge",
        "beyond-systemd-span",
    ] {
        let error = ruxel_config::eval_config(
            &root()
                .join("invalid")
                .join(format!("timer-boot-{name}.pkl")),
            &options(),
        )
        .err()
        .ok_or("evaluated")?
        .to_string();
        assert!(
            error.contains("onBoot is beyond the 9007199254740991ms"),
            "{name}: {error}"
        );
    }
    Ok(())
}

/// Two spellings of one delay are one timer.
#[test]
fn two_spellings_of_one_delay_are_one_timer() -> Result {
    let config = valid("systemd-timer-delay-spellings.pkl")?;
    let all = tasks(&config);
    let timers: Vec<&&serde_json::Value> = all
        .iter()
        .filter(|task| {
            task["operation"]["dest"] == "/etc/systemd/system/postgresql-spelled-delay.timer"
        })
        .collect();
    assert_eq!(timers.len(), 1, "{timers:#?}");
    let content = timers[0]["operation"]["content"]
        .as_str()
        .ok_or("no timer file content")?;
    // The delay keeps the millisecond it was written with, whether it was
    // written in seconds or in milliseconds.
    assert!(content.contains("OnBootSec=1001ms"), "{content}");
    Ok(())
}

/// A whole-millisecond delay keeps every digit at every magnitude.
#[test]
fn an_exactly_integral_boot_delay_keeps_every_digit() -> Result {
    let config = valid("systemd-timer-exact-huge-delay.pkl")?;
    let all = tasks(&config);
    for (unit, on_boot) in [
        ("exact-millisecond", "OnBootSec=4503599627370497ms"),
        ("exact-half-even", "OnBootSec=4503599627370497ms"),
        ("exact-odd-bound", "OnBootSec=9007199254740991ms"),
    ] {
        let dest = format!("/etc/systemd/system/postgresql-{unit}.timer");
        let content = all
            .iter()
            .find(|task| task["operation"]["dest"] == dest)
            .and_then(|task| task["operation"]["content"].as_str())
            .ok_or_else(|| format!("no timer file {dest}"))?;
        assert!(content.contains(on_boot), "{dest}: {content}");
    }
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
    Ok(())
}

/// A timer basename may be 249 characters beside the suffix.
#[test]
fn a_timer_basename_may_be_249_characters() -> Result {
    let config = valid("systemd-timer-long-name.pkl")?;
    let all = tasks(&config);
    // The basename the corpus case writes: a `t` and 248 `a`s.
    let basename: String = "t".to_owned() + &"a".repeat(248);
    assert_eq!(basename.len(), 249);
    // The unit file the timer owns carries the basename the author wrote.
    let dest = format!("/etc/systemd/system/{basename}.timer");
    let timer = all
        .iter()
        .find(|task| task["operation"]["dest"] == dest)
        .ok_or("no timer file")?;
    assert_eq!(timer["operation"]["dest"], dest);
    // The unit name systemd reads is the basename with its suffix, and it
    // fits systemd's own limit of 255 characters.
    assert_eq!(basename.len() + ".timer".len(), 255);
    Ok(())
}

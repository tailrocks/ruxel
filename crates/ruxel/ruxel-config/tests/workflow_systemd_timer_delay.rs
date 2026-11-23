//! The timer delay contract of the workflow language: the whole-millisecond
//! precision the boot delay and the repeat keep, and the limits they refuse.
//!
//! The timer assertions themselves live in `workflow_systemd.rs`; this file
//! holds the delay cases, which the size policy keeps apart from it.

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

/// A negative, zero, or fractional repeat is refused, not rounded or sign-flipped.
#[test]
fn a_bad_repeat_is_refused() -> Result {
    for (name, expected) in [
        (
            "timer-repeat-negative.pkl",
            "'The impatient check': repeat -5.min is negative; \
             a timer fires after its last firing, not before it",
        ),
        (
            "timer-repeat-zero.pkl",
            "'The idle check': repeat 0.s is zero; \
             a timer fires after its last firing, not at it",
        ),
        (
            "timer-repeat-fractional-millisecond.pkl",
            "'The sloppy check': repeat 1000.0000005.ms must be a whole number of milliseconds; \
             a systemd timer reads the interval in seconds or milliseconds",
        ),
    ] {
        let path = root().join("invalid").join(name);
        let error = ruxel_config::eval_config(&path, &options())
            .err()
            .ok_or_else(|| format!("{name}: evaluated"))?
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
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
    let daily = content("/etc/systemd/system/postgresql-multixact-monitor.timer")?;
    assert!(daily.contains("OnBootSec=120"), "{daily}");
    assert!(daily.contains("OnUnitActiveSec=300"), "{daily}");
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
    // A half millisecond is refused: the contract is whole-millisecond.
    let half = root().join("invalid/systemd-timer-fractional-delay.pkl");
    let error = ruxel_config::eval_config(&half, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    assert!(
        error.contains("must be a whole number of milliseconds"),
        "{error}"
    );
    let lost = root().join("invalid").join("timer-boot-lost-fraction.pkl");
    let error = ruxel_config::eval_config(&lost, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    assert!(
        error.contains("must be a whole number of milliseconds"),
        "{error}"
    );
    Ok(())
}

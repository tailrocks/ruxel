//! The second apply replays the later stage with fresh agent state:
//! the plan declares only the replayed stage, and the ledger's carry
//! of the earlier stage is what admits the preflight — the plan alone
//! would refuse the leftover assignment as unmanaged.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::super::boot;

const KEY: &str = "net.ipv4.ip_forward";
const NEW: &str = "/etc/sysctl.d/10-new.conf";
const OLD: &str = "/etc/sysctl.d/99-old.conf";
const ETC: &str = "/etc/sysctl.d/50-ruxel.conf";
const USR: &str = "/usr/lib/sysctl.d/50-ruxel.conf";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-fallback-{name}-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    dir
}

fn seed(dir: &Path, key: &str, file: &str, value: &str) -> crate::ledger::Ledger {
    let mut ledger = crate::ledger::Ledger::load(dir);
    ledger.record(
        key,
        "sysctl",
        &json!({"name": KEY, "value": value, "sysctl_file": file}),
        "ok",
        &json!({"changed": true}),
    );
    ledger.flush();
    crate::ledger::Ledger::load(dir)
}

fn host(pairs: &[(&str, &str)]) -> Vec<(PathBuf, Vec<u8>)> {
    pairs
        .iter()
        .map(|(path, content)| (PathBuf::from(*path), content.as_bytes().to_vec()))
        .collect()
}

#[test]
fn a_replayed_stage_admits_through_the_carry_over() {
    // The first apply declared the key in the later-loading file; the
    // replay's plan declares only the earlier-loading one. The leftover
    // assignment is unmanaged under the plan's own set and a declared
    // transition with the ledger's carry behind it.
    let dir = scratch("override");
    let ledger = seed(&dir, "earlier", OLD, "0");
    let planned = super::super::SysctlManaged::testing(vec![super::super::ManagedSysctl::plain(
        NEW.to_string(),
        KEY.to_string(),
        "1".to_string(),
    )]);
    let host = host(&[(OLD, "net.ipv4.ip_forward=0\n")]);
    assert_eq!(
        boot::boot_override(NEW, KEY, "1", &host, &planned),
        Some((OLD.to_string(), "0".to_string())),
        "the plan alone refuses the leftover: the reported failure"
    );
    let mut merged = planned.clone();
    merged.entries.extend(
        ledger
            .sysctl_preflight_managed("sysctl", &planned.entries)
            .entries,
    );
    // Thread 332: the carry-over no longer admits the replay — the
    // retained record of the earlier revision's later file is history,
    // so the leftover refuses even beside the plan's own entries. Only
    // the plan's declarations waive.
    assert_eq!(
        boot::boot_override(NEW, KEY, "1", &host, &merged),
        Some((OLD.to_string(), "0".to_string())),
        "the carry-over admits nothing: the leftover refuses"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_replayed_shadow_target_admits_through_the_carry_over() {
    // The same carry in the shadow direction: the first apply declared
    // the shadowing file, the replay's plan only the shadowed target.
    let dir = scratch("shadow");
    let ledger = seed(&dir, "earlier", ETC, "1");
    let planned = super::super::SysctlManaged::testing(vec![super::super::ManagedSysctl::plain(
        USR.to_string(),
        KEY.to_string(),
        "1".to_string(),
    )]);
    assert!(
        !super::super::declared_shadow(ETC, KEY, &planned),
        "the plan alone leaves the leftover shadow unmanaged"
    );
    let mut merged = planned.clone();
    merged.entries.extend(
        ledger
            .sysctl_preflight_managed("sysctl", &planned.entries)
            .entries,
    );
    assert!(
        super::super::declared_shadow(ETC, KEY, &merged),
        "the carry-over declares the shadow's managed value"
    );
    drop(std::fs::remove_dir_all(&dir));
}

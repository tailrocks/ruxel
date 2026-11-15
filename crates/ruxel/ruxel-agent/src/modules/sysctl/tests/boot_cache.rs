//! The cache gate: a cached sysctl result stands only while the
//! current managed set still authorizes the boot surface. The
//! fingerprint pins the files' bytes, not who manages them: dropping a
//! later-loading task leaves its file behind, and the bytes — and with
//! them every probe — still match. The gate replays the override
//! preflight under the current declarations, so the dropped task
//! invalidates the earlier task's cache and the task runs (and refuses)
//! instead of reporting convergence it cannot survive reboot with.

use std::path::PathBuf;

use super::super::{sysctl_cache_covered, sysctl_key};

const KEY: &str = "net.ipv4.ip_forward";
const EARLY: &str = "/etc/sysctl.d/10-new.conf";
const LATE: &str = "/etc/sysctl.d/99-old.conf";

/// The boot surface of the replay: the earlier task's file assigns the
/// managed value, the dropped task's leftover assigns another value
/// from the later-loading file.
fn host() -> Vec<(PathBuf, Vec<u8>)> {
    vec![(PathBuf::from(LATE), b"net.ipv4.ip_forward=0\n".to_vec())]
}

#[test]
fn a_retained_declaration_keeps_the_cache_covered() {
    // Run 1: the removed task's retained declaration still authorizes
    // its file — the one-run bounded window the plan-first preflight
    // documents — so the earlier task is served from the cache.
    let managed = vec![
        (EARLY.to_string(), KEY.to_string(), "1".to_string()),
        (LATE.to_string(), KEY.to_string(), "0".to_string()),
    ];
    assert!(
        sysctl_cache_covered(EARLY, KEY, "1", &host(), &managed),
        "the retained declaration authorizes the leftover for exactly one run"
    );
    assert_eq!(sysctl_key(KEY), "net/ipv4/ip_forward");
}

#[test]
fn a_dropped_declaration_unveils_the_cache() {
    // Run 2: the plan's end pruned the removed task, so its file is
    // unmanaged now. The surface is byte-identical — every probe still
    // matches — but the boot value comes from the leftover, and the
    // cache must miss so the task runs and its preflight refuses.
    let managed = vec![(EARLY.to_string(), KEY.to_string(), "1".to_string())];
    assert!(
        !sysctl_cache_covered(EARLY, KEY, "1", &host(), &managed),
        "a dropped task's leftover must invalidate the earlier task's cache"
    );
}

#[test]
fn a_later_assignment_of_the_same_value_keeps_the_cache_covered() {
    // An unmanaged later file assigning the same value hands boot the
    // managed value — not an override — so the cache stands.
    let host = vec![(PathBuf::from(LATE), b"net.ipv4.ip_forward=1\n".to_vec())];
    let managed = vec![(EARLY.to_string(), KEY.to_string(), "1".to_string())];
    assert!(
        sysctl_cache_covered(EARLY, KEY, "1", &host, &managed),
        "the same boot value is not an override"
    );
}

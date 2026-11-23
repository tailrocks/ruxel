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
    let managed = super::super::SysctlManaged::testing(vec![
        super::super::ManagedSysctl::plain(EARLY.to_string(), KEY.to_string(), "1".to_string()),
        super::super::ManagedSysctl::plain(LATE.to_string(), KEY.to_string(), "0".to_string()),
    ]);
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
    let managed = super::super::SysctlManaged::testing(vec![super::super::ManagedSysctl::plain(
        EARLY.to_string(),
        KEY.to_string(),
        "1".to_string(),
    )]);
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
    let managed = super::super::SysctlManaged::testing(vec![super::super::ManagedSysctl::plain(
        EARLY.to_string(),
        KEY.to_string(),
        "1".to_string(),
    )]);
    assert!(
        sysctl_cache_covered(EARLY, KEY, "1", &host, &managed),
        "the same boot value is not an override"
    );
}

#[test]
fn an_undeclared_shadow_unveils_the_cache() {
    // Thread 353: the cache gate replayed only the override, which
    // weighs solely later-loading files — so a same-basename shadow in
    // a higher-precedence directory bypassed the gate forever after
    // the transition run, reporting convergence while the boot value
    // came from the shadow's file. The gate must replay the shadow
    // check against the current managed set too.
    use super::boot_surface::Scratch;

    let scratch = Scratch::new("cache-shadow");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let managed_path = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let managed_text = managed_path.to_str().expect("path is text").to_owned();
    std::fs::write(etc.join("50-ruxel.conf"), "net.ipv4.ip_forward=0\n")
        .expect("operation succeeds");
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    let managed = super::super::SysctlManaged::testing(vec![super::super::ManagedSysctl::plain(
        managed_text.clone(),
        KEY.to_string(),
        "1".to_string(),
    )]);
    assert!(
        !super::super::sysctl_cache_covered_in(&managed_text, KEY, "1", &[], &managed, &dirs),
        "a shadow nothing declares must unveil the cache"
    );
}

#[test]
fn a_declared_shadow_keeps_the_cache_covered() {
    // The deliberate transition of the key to the higher-precedence
    // file declares the shadow as a managed value of its own, so the
    // shadow is the workflow's and the cache stands.
    use super::boot_surface::Scratch;

    let scratch = Scratch::new("cache-declared-shadow");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let managed_path = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let managed_text = managed_path.to_str().expect("path is text").to_owned();
    let shadow_path = etc.join("50-ruxel.conf");
    std::fs::write(&shadow_path, "net.ipv4.ip_forward=1\n").expect("operation succeeds");
    let shadow_text = shadow_path.to_str().expect("path is text").to_owned();
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    let managed = super::super::SysctlManaged::testing(vec![
        super::super::ManagedSysctl::plain(managed_text.clone(), KEY.to_string(), "1".to_string()),
        super::super::ManagedSysctl::plain(shadow_text, KEY.to_string(), "1".to_string()),
    ]);
    assert!(
        super::super::sysctl_cache_covered_in(&managed_text, KEY, "1", &[], &managed, &dirs),
        "a declared shadow is the workflow's own transition"
    );
}

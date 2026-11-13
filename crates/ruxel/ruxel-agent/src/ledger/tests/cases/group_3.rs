use super::*;

#[cfg(unix)]
#[test]
fn cached_ok_hits_when_directory_attrs_unchanged() {
    let dir = Scratch::new("dir-hit");
    let target = dir.0.join("target");
    std::fs::create_dir(&target).expect("operation succeeds");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &target);
    assert!(ledger.cached_ok("key").is_some());
}

#[cfg(unix)]
#[test]
fn cached_ok_misses_when_directory_mode_changes() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Scratch::new("dir-drift");
    let target = dir.0.join("target");
    std::fs::create_dir(&target).expect("operation succeeds");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &target);
    let old_mode = std::fs::metadata(&target)
        .expect("operation succeeds")
        .permissions()
        .mode()
        & 0o7777;
    let new_mode = if old_mode == 0o700 { 0o755 } else { 0o700 };
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(new_mode))
        .expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());
}

#[test]
fn sysctl_live_verification_normalizes_and_rejects_missing_values() {
    assert_eq!(normalized("1\t2  3"), normalized("1 2 3"));
    assert!(
        !Probe::SysctlLive {
            name: "ruxel.test.nonexistent".to_string(),
            value: "1".to_string(),
        }
        .verify(&mut crate::system_state::SystemState::default())
    );
}

#[test]
fn sysctl_cache_misses_for_later_duplicate_assignments() {
    let dir = Scratch::new("sysctl-duplicate");
    let file = dir.file("sysctl.conf", b"# net.test = 8\nnet.test = 1\n");
    let params = json!({"name": "net.test", "value": "1", "sysctl_file": file});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("key", "sysctl", &params, "ok", &json!({"changed": true}));
    assert!(ledger.cached_ok("key").is_some());

    std::fs::write(&file, b"# net.test = 8\nnet.test = 1\nnet.test = 2\n")
        .expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());

    std::fs::write(&file, b"net.test = 1\nnet.test = 1\n").expect("operation succeeds");
    ledger.record(
        "duplicate",
        "sysctl",
        &params,
        "ok",
        &json!({"changed": true}),
    );
    assert!(ledger.cached_ok("duplicate").is_none());
}

#[test]
fn sysctl_alias_drift_misses_the_cache() {
    // One parameter's identity is its /proc/sys path: the slash-first
    // and dotted spellings (`net/ipv4/ip_forward`,
    // `net.ipv4.ip_forward`) are one entry. The probe must weigh every
    // alias line in the file — pinning only the raw spelling the task
    // requested lets an alias that drifted to another value pass every
    // probe, the task is skipped, and boot processes the alias and
    // reverts the parameter.
    let dir = Scratch::new("sysctl-alias-drift");
    let file = dir.file("sysctl.conf", b"net.ipv4.ip_forward=1\n");
    let params = json!({"name": "net.ipv4.ip_forward", "value": "1", "sysctl_file": file});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("key", "sysctl", &params, "ok", &json!({"changed": true}));
    assert!(ledger.cached_ok("key").is_some());

    std::fs::write(&file, b"net.ipv4.ip_forward=1\nnet/ipv4/ip_forward=0\n")
        .expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());
}

#[test]
fn sysctl_probes_weigh_every_alias_spelling() {
    // An alias at another value leaves no single value to pin (the
    // later alias restores its own at boot), and the alias spelling
    // alone is the same entry: the probe records the task's value
    // against it instead of matching nothing.
    let dir = Scratch::new("sysctl-alias-probe");
    let file = dir.file(
        "sysctl.conf",
        b"net.ipv4.ip_forward=1\nnet/ipv4/ip_forward=0\n",
    );
    assert_eq!(
        sysctl_file_value(
            file.to_str().expect("operation succeeds"),
            "net.ipv4.ip_forward"
        ),
        None
    );
    let alias_only = dir.file("alias.conf", b"net/ipv4/ip_forward=1\n");
    assert_eq!(
        sysctl_file_value(
            alias_only.to_str().expect("operation succeeds"),
            "net.ipv4.ip_forward"
        ),
        Some("1".to_string())
    );
}

#[test]
fn the_sysctl_cache_weighs_the_boot_surface() {
    // The boot surface decides the value the host comes up with: an
    // unmanaged file added or changed after the task ran moves the boot
    // value the task never judged, so the cached result must not be
    // served. The surface is one of the probes, and the probe compares
    // against the surface of the moment.
    let dir = Scratch::new("sysctl-boot-surface");
    let file = dir.file("sysctl.conf", b"net.test = 1\n");
    let params = json!({"name": "net.test", "value": "1", "sysctl_file": file});
    let mut ledger = Ledger::load(&dir.0);
    ledger.record("key", "sysctl", &params, "ok", &json!({"changed": true}));
    assert!(ledger.cached_ok("key").is_some());

    let stale = Probe::SysctlBoot {
        fingerprint: "a surface that never was".to_string(),
    };
    assert!(
        !stale.verify(&mut crate::system_state::SystemState::default()),
        "a boot surface that moved must miss the cache"
    );
    let current = crate::modules::boot_fingerprint().expect("fingerprint");
    let fresh = Probe::SysctlBoot {
        fingerprint: current.clone(),
    };
    assert!(
        fresh.verify(&mut crate::system_state::SystemState::default()),
        "the surface of the moment is the one the probe weighs"
    );
}

#[test]
fn the_sysctl_probe_reads_a_dash_prefixed_assignment() {
    // procps reads a leading '-' as the ignore-failure marker: the
    // assignment still happens, so the line is the entry. A probe that
    // missed it would pin no value for the file, and a file that holds
    // the marked assignment beside the plain one would restore the
    // marked value at boot.
    let dir = Scratch::new("sysctl-dash-probe");
    let file = dir.file("sysctl.conf", b"-net.ipv4.ip_forward=1\n");
    assert_eq!(
        sysctl_file_value(
            file.to_str().expect("operation succeeds"),
            "net.ipv4.ip_forward"
        ),
        Some("1".to_string())
    );
}

#[test]
fn old_schema_ledger_loads_empty() {
    let dir = Scratch::new("old-schema");
    let file = dir.file("target", b"stable");
    std::fs::create_dir_all(dir.0.join("ledger")).expect("operation succeeds");
    let old = json!({
        "key": {
            "agent_version": env!("CARGO_PKG_VERSION"),
            "status": "ok",
            "result_json": {"changed": false},
            "probes": [{
                "kind": "File",
                "path": file,
                "sha256": "unused",
                "len": 6
            }]
        }
    });
    std::fs::write(
        dir.0.join("ledger/ledger.json"),
        serde_json::to_vec(&old).expect("operation succeeds"),
    )
    .expect("operation succeeds");
    assert!(Ledger::load(&dir.0).cached_ok("key").is_none());
}

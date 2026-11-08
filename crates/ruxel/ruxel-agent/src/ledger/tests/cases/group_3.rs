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

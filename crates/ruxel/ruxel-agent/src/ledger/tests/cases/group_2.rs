use super::*;

#[cfg(unix)]
#[test]
fn cached_ok_misses_when_file_is_replaced_by_symlink() {
    let dir = Scratch::new("symlink-drift");
    let file = dir.file("target", b"stable");
    let replacement = dir.file("replacement", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    std::fs::remove_file(&file).expect("operation succeeds");
    std::os::unix::fs::symlink(replacement, &file).expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());
}

#[test]
fn cached_ok_misses_on_agent_version_change() {
    let dir = Scratch::new("version-drift");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    ledger.flush();

    let path = dir.0.join("ledger/ledger.json");
    let mut stored: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("operation succeeds"))
            .expect("operation succeeds");
    stored["key"]["agent_version"] = json!("0.0.0-test");
    std::fs::write(
        path,
        serde_json::to_vec(&stored).expect("operation succeeds"),
    )
    .expect("operation succeeds");
    assert!(Ledger::load(&dir.0).cached_ok("key").is_none());
}

#[cfg(unix)]
#[test]
fn cached_ok_misses_when_mode_changed() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Scratch::new("mode-drift");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    let old_mode = std::fs::metadata(&file)
        .expect("operation succeeds")
        .permissions()
        .mode()
        & 0o7777;
    let new_mode = if old_mode == 0o600 { 0o644 } else { 0o600 };
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(new_mode))
        .expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());
}

#[test]
fn record_skips_failed_and_skipped() {
    let dir = Scratch::new("status-gate");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    for status in ["failed", "skipped"] {
        ledger.record(status, "file", &json!({"path": file}), status, &json!({}));
        assert!(ledger.cached_ok(status).is_none());
    }
}

#[test]
fn record_skips_noncacheable_modules() {
    let dir = Scratch::new("honesty-gate");
    let mut ledger = Ledger::load(&dir.0);
    for (key, module, params) in [
        ("command", "command", json!({})),
        ("shell", "shell", json!({})),
        (
            "restart",
            "systemd",
            json!({"name": "unused", "state": "restarted"}),
        ),
    ] {
        ledger.record(key, module, &params, "ok", &json!({}));
        assert!(ledger.cached_ok(key).is_none());
    }
}

#[test]
fn record_skips_empty_key() {
    let dir = Scratch::new("empty-key");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "", &file);
    assert!(ledger.records.is_empty());
}

#[test]
fn apt_latest_is_not_cached() {
    let params = json!({"name": "not-queried", "state": "latest"});
    assert!(
        probe_for(
            "apt",
            &params,
            &mut crate::system_state::SystemState::default()
        )
        .is_none()
    );
}

#[test]
fn explicit_refreshes_and_actions_bypass_even_existing_records() {
    let dir = Scratch::new("action-cache-gate");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "old-record", &file);
    assert!(ledger.cached_ok("old-record").is_some());

    for (module, params) in [
        ("apt", json!({"name":"pkg", "update_cache":true})),
        ("apt", json!({"name":"pkg", "autoremove":"yes"})),
        ("apt", json!({"name":"pkg", "upgrade":"dist"})),
        ("apt", json!({"name":"pkg", "state":"latest"})),
        ("systemd", json!({"name":"unit", "daemon_reload":true})),
        ("systemd", json!({"name":"unit", "daemon_reload":"yes"})),
        ("systemd", json!({"name":"unit", "state":"restarted"})),
    ] {
        assert!(!cache_allowed(module, &params), "{module} {params}");
        ledger.record("new-record", module, &params, "ok", &json!({}));
        assert!(ledger.cached_ok("new-record").is_none());
    }
    assert!(cache_allowed(
        "apt",
        &json!({"name":"pkg", "update_cache":false})
    ));
    assert!(cache_allowed(
        "systemd",
        &json!({"name":"unit", "daemon_reload":false})
    ));
}

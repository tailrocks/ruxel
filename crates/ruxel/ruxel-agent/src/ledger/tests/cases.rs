use super::super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ruxel-ledger-test-{}-{name}-{id}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("operation succeeds");
        Self(path)
    }

    fn file(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, content).expect("operation succeeds");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn record_file(ledger: &mut Ledger, key: &str, path: &Path) {
    ledger.record(
        key,
        "file",
        &json!({"path": path}),
        "ok",
        &json!({"changed": true, "marker": "kept"}),
    );
}

#[test]
fn load_missing_ledger_is_empty() {
    let dir = Scratch::new("missing");
    assert!(Ledger::load(&dir.0).cached_ok("missing").is_none());
}

#[test]
fn flush_noop_when_not_dirty() {
    let dir = Scratch::new("flush-noop");
    Ledger::load(&dir.0).flush();
    assert!(!dir.0.join("ledger/ledger.json").exists());
}

#[test]
fn record_then_flush_then_load_roundtrips() {
    let dir = Scratch::new("roundtrip");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    ledger.flush();

    let result = Ledger::load(&dir.0)
        .cached_ok("key")
        .expect("operation succeeds");
    assert_eq!(result["changed"], false);
    assert_eq!(result["marker"], "kept");
}

#[cfg(unix)]
#[test]
fn ledger_files_and_directory_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Scratch::new("private");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "controller-key", &file);
    ledger.flush();

    let ledger_dir = dir.0.join("ledger");
    assert_eq!(
        std::fs::metadata(&ledger_dir)
            .expect("operation succeeds")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for path in [ledger_dir.join("key"), ledger_dir.join("ledger.json")] {
        assert_eq!(
            std::fs::metadata(path)
                .expect("operation succeeds")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn stored_identity_is_host_keyed_and_stable() {
    let dir = Scratch::new("keyed");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "controller-key", &file);
    ledger.flush();

    let stored: Value = serde_json::from_slice(
        &std::fs::read(dir.0.join("ledger/ledger.json")).expect("operation succeeds"),
    )
    .expect("operation succeeds");
    let stored_key = stored
        .as_object()
        .expect("operation succeeds")
        .keys()
        .next()
        .expect("operation succeeds");
    assert_ne!(stored_key, "controller-key");
    assert_eq!(stored_key.len(), 64);
    assert!(Ledger::load(&dir.0).cached_ok("controller-key").is_some());
}

#[test]
fn corrupt_ledger_json_loads_empty() {
    let dir = Scratch::new("corrupt");
    std::fs::create_dir_all(dir.0.join("ledger")).expect("operation succeeds");
    std::fs::write(dir.0.join("ledger/ledger.json"), b"not json").expect("operation succeeds");
    assert!(Ledger::load(&dir.0).cached_ok("key").is_none());
}

#[test]
fn cached_ok_hits_when_content_unchanged() {
    let dir = Scratch::new("hit");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    assert_eq!(
        ledger.cached_ok("key").expect("operation succeeds")["changed"],
        false
    );
}

#[test]
fn cached_ok_misses_when_content_changed() {
    let dir = Scratch::new("content-drift");
    let file = dir.file("target", b"before");
    let mut ledger = Ledger::load(&dir.0);
    record_file(&mut ledger, "key", &file);
    std::fs::write(&file, b"after").expect("operation succeeds");
    assert!(ledger.cached_ok("key").is_none());
}

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

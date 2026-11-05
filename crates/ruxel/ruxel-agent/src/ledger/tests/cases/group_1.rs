use super::*;

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

use super::backend::ProcessBackend;
use super::*;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// A fake fnox for the store fixture: values live in files beside the
/// script, reads go through `fnox get --if-missing error` (the store's read
/// path), `set` reads the value from stdin (never argv) and counts its
/// calls, and a missing key exits 1 with `not found`. No real secret value
/// is ever read: the fixture writes its own.
struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "ruxel-secret-generation-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create fixture directory");
        let script = directory.join("fnox");
        fs::write(
            &script,
            r#"#!/bin/sh
store="${0%/*}/store"
calls="${0%/*}/calls"
if [ "$1" = "get" ]; then
  [ "$#" -eq 4 ] && [ "$2" = "--if-missing" ] && [ "$3" = "error" ] || { echo "get takes --if-missing error and the key" >&2; exit 7; }
  key="$4"
  if [ -f "$store/$key" ]; then
    cat "$store/$key"
    exit 0
  fi
  echo "secret $key not found" >&2
  exit 1
fi
if [ "$1" = "set" ]; then
  [ "$#" -eq 2 ] || { echo "set takes only the key argument" >&2; exit 9; }
  printf 'set\n' >> "$calls"
  key="$2"
  mkdir -p "$store"
  cat > "$store/$key"
  exit 0
fi
exit 8
"#,
        )
        .expect("write fixture script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("mode script");
        Self { directory }
    }

    fn backend(&self) -> ProcessBackend {
        ProcessBackend::with_programs(self.directory.join("fnox"), "/nonexistent/ruxel-op")
            .with_lock_dir(&self.directory)
    }

    fn resolver(&self) -> Resolver {
        Resolver::new(Arc::new(self.backend())).unwrap()
    }

    fn set_calls(&self) -> usize {
        fs::read_to_string(self.directory.join("calls")).map_or(0, |calls| calls.lines().count())
    }

    fn value(&self, key: &str) -> Option<String> {
        fs::read_to_string(self.directory.join("store").join(key)).ok()
    }

    fn put(&self, key: &str, value: &str) {
        let store = self.directory.join("store");
        drop(fs::create_dir_all(&store));
        fs::write(store.join(key), value).expect("write fixture value");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.directory);
    }
}

fn request(key: &str, format: &str) -> GeneratedRequest {
    GeneratedRequest {
        key: key.to_owned(),
        format: format.to_owned(),
    }
}

/// The twice-apply guarantee: the same workflow applied twice creates the
/// key exactly once. The first run generates, stores on stdin, reads back
/// and compares; the second finds the key present and stores nothing.
#[test]
fn applying_the_same_workflow_twice_generates_exactly_once() {
    let fixture = Fixture::new();
    let requests = [request("PROCESSOR_DB_PASSWORD", "hex32")];
    let mut generated = 0;
    fixture
        .resolver()
        .ensure_generated(&requests, &mut |_, _, _| generated += 1)
        .expect("first apply generates");
    assert_eq!(fixture.set_calls(), 1);
    assert_eq!(generated, 1);
    let first = fixture.value("PROCESSOR_DB_PASSWORD").expect("stored");
    // The second apply, a fresh run over the same store, generates nothing.
    let mut generated = 0;
    fixture
        .resolver()
        .ensure_generated(&requests, &mut |_, _, _| generated += 1)
        .expect("second apply finds the key present");
    assert_eq!(fixture.set_calls(), 1, "generated exactly once");
    assert_eq!(generated, 0);
    assert_eq!(
        fixture.value("PROCESSOR_DB_PASSWORD").as_deref(),
        Some(&*first)
    );
}

/// A key that appears between the check and the store fails the run rather
/// than being overwritten: the store it appeared in keeps its value.
#[test]
fn a_key_that_appears_between_check_and_store_fails_without_overwriting() {
    struct Appearing {
        reads: AtomicUsize,
    }
    impl Backend for Appearing {
        fn fnox_get(&self, key: &str) -> Result<Option<String>, String> {
            // Missing on the presence check, present on the second look:
            // someone else created it without honoring the lock.
            if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(None)
            } else {
                Ok(Some(format!("written by someone else, not {key}")))
            }
        }
        fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
            panic!("the run must fail before it overwrites");
        }
        fn onepassword_fields(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            unreachable!()
        }
        fn onepassword_reference(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            unreachable!()
        }
    }
    let resolver = Resolver::new(Arc::new(Appearing {
        reads: AtomicUsize::new(0),
    }))
    .unwrap();
    let error = resolver
        .ensure_generated(
            &[request("PROCESSOR_DB_PASSWORD", "hex32")],
            &mut |_, _, _| {},
        )
        .expect_err("the run fails");
    assert!(
        error.contains("appeared in fnox"),
        "names the race: {error}"
    );
    assert!(!error.contains("written by someone else"));
}

/// A read-back that differs from what was stored fails the run: the value in
/// fnox is what the run registered, or the run does not proceed.
#[test]
fn a_read_back_that_differs_fails_the_run() {
    struct Tampering {
        reads: AtomicUsize,
    }
    impl Backend for Tampering {
        fn fnox_get(&self, key: &str) -> Result<Option<String>, String> {
            // Missing for the presence check and the appearance check; the
            // read-back after the store finds something else.
            if self.reads.fetch_add(1, Ordering::SeqCst) < 2 {
                Ok(None)
            } else {
                Ok(Some(format!("tampered after set ({key})")))
            }
        }
        fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn onepassword_fields(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            unreachable!()
        }
        fn onepassword_reference(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            unreachable!()
        }
    }
    let resolver = Resolver::new(Arc::new(Tampering {
        reads: AtomicUsize::new(0),
    }))
    .unwrap();
    let error = resolver
        .ensure_generated(
            &[request("PROCESSOR_DB_PASSWORD", "hex32")],
            &mut |_, _, _| {},
        )
        .expect_err("read-back compare fails the run");
    assert!(
        error.contains("read back from fnox differs"),
        "names the comparison: {error}"
    );
    assert!(!error.contains("tampered"), "errors never carry values");
}

/// The store round-trips the value through the real fixture: `set` receives
/// the value on stdin only, and the read back matches byte for byte.
#[test]
fn the_fixture_store_round_trips_stdin_values() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    let requests = [
        request("HEX_KEY", "hex16"),
        request("ALNUM_KEY", "alnum32"),
        request("B64_KEY", "base64-32"),
    ];
    let mut created = Vec::new();
    resolver
        .ensure_generated(&requests, &mut |key, _, _| created.push(key.to_owned()))
        .expect("generate");
    assert_eq!(created, ["HEX_KEY", "ALNUM_KEY", "B64_KEY"]);
    assert_eq!(fixture.set_calls(), 3);
    for request in &requests {
        let stored = fixture.value(&request.key).expect("stored in the fixture");
        match request.format.as_str() {
            "hex16" => assert_eq!(stored.len(), 16),
            "alnum32" => assert_eq!(stored.len(), 32),
            "base64-32" => assert_eq!(stored.len(), 44),
            _ => panic!("unexpected format"),
        }
    }
    // The key is on argv for `set`, the value never is: the script refuses
    // any second argument, so a run that tried it would fail, not leak.
    assert_eq!(fixture.set_calls(), 3);
}

/// The fetched secret is stored only when it differs from the stored value,
/// compared on the controller; the fingerprint it reports is eight hex
/// characters and keyed, never an unsalted full hash.
#[test]
fn a_fetched_secret_stores_only_a_change_and_fingerprints_are_keyed_and_truncated() {
    let fixture = Fixture::new();
    fixture.put("K8S_ADMIN_KUBECONFIG", "stored kubeconfig content\n");
    let resolver = fixture.resolver();
    // Same content: no store, no change.
    let (changed, fingerprint) = resolver
        .fetch_secret("K8S_ADMIN_KUBECONFIG", "stored kubeconfig content\n")
        .expect("compare");
    assert!(!changed);
    assert_eq!(
        fingerprint.len(),
        8,
        "the unchanged store reports a fingerprint too"
    );
    assert_eq!(fixture.set_calls(), 0);
    // Different content: one store, a change, an eight-hex fingerprint.
    let (changed, fingerprint) = resolver
        .fetch_secret("K8S_ADMIN_KUBECONFIG", "new kubeconfig content\n")
        .expect("store");
    assert!(changed);
    assert_eq!(fixture.set_calls(), 1);
    assert_eq!(
        fixture.value("K8S_ADMIN_KUBECONFIG").as_deref(),
        Some("new kubeconfig content\n")
    );
    assert_eq!(fingerprint.len(), 8);
    assert!(
        fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    // The fingerprint is keyed per process: another resolver's digest of the
    // same value differs, so no offline oracle can test guesses against it.
    let other = fixture
        .resolver()
        .fingerprint("K8S_ADMIN_KUBECONFIG", "new kubeconfig content\n");
    assert_ne!(other, fingerprint);
    // A malformed key is refused before any store contact.
    assert!(resolver.fetch_secret("k8s_admin_kubeconfig", "x").is_err());
    assert_eq!(fixture.set_calls(), 1);
}

/// Synthetic mode generates nothing and answers placeholders: the resolver
/// refuses the generate preflight outright.
#[test]
fn synthetic_mode_never_generates() {
    let resolver = Resolver::synthetic().unwrap();
    assert!(
        resolver
            .ensure_generated(
                &[request("PROCESSOR_DB_PASSWORD", "hex32")],
                &mut |_, _, _| {}
            )
            .is_err()
    );
}

/// Every generated lookup of a configuration reaches the preflight, once
/// per key, in field order.
#[test]
fn generated_requests_are_collected_from_the_configuration() {
    let config = json!({
        "plays": [{"tasks": [
            {"operation": {"kind": "copy", "content": {"kind": "concat", "args": [
                {"kind": "lookup", "provider": "fnox-generated", "key": "PROCESSOR_DB_PASSWORD", "format": "hex32"},
                {"kind": "lookup", "provider": "fnox", "key": "PLAIN", "argv": []},
                {"kind": "lookup", "provider": "fnox-generated", "key": "PROCESSOR_DB_PASSWORD", "format": "hex32"},
                {"kind": "lookup", "provider": "fnox-generated", "key": "OTHER_KEY", "format": "alnum32"}
            ]}}}
        ]}]
    });
    let requests = generated_requests(&config);
    assert_eq!(
        requests,
        [
            request("PROCESSOR_DB_PASSWORD", "hex32"),
            request("OTHER_KEY", "alnum32"),
        ]
    );
}

/// The write lock is real: a second holder waits until the first releases.
#[test]
fn the_write_lock_serializes_holders() {
    let directory = std::env::temp_dir().join(format!(
        "ruxel-fnox-lock-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default(),
    ));
    fs::create_dir_all(&directory).expect("create lock directory");
    let backend = ProcessBackend::with_programs("fnox", "op").with_lock_dir(&directory);
    let first = backend.fnox_write_lock("KEY").expect("first lock");
    let started = std::time::Instant::now();
    let holder = std::thread::spawn(move || {
        let _lock = first;
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    holder.join().expect("holder thread");
    let second = backend.fnox_write_lock("KEY").expect("second lock");
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(150),
        "the second lock waited for the first to release"
    );
    drop(second);
    let _removed = fs::remove_dir_all(&directory);
}

use super::backend::ProcessBackend;
use super::*;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "generation/fetch.rs"]
mod fetch;
#[path = "generation/races.rs"]
mod races;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// A fake fnox for the store fixture: values live in files beside the
/// script, reads go through `fnox get --if-missing error` (the store's read
/// path), `set` takes the value from a file — `--from-file`, the
/// byte-preserving store of the pinned CLI, which reads the file's exact
/// bytes — and counts its calls; the stdin form is modelled too, and it
/// trims the way the pinned CLI trims. A missing key exits 1 with
/// `not found`. The store's own question about the key — `env --json
/// --describe`, the unfiltered query — lists every key of the active
/// profile with its `env` and `as_file` flags; the fixture's keys hold
/// plain values, never file-backed ones. No real secret value is ever
/// read: the fixture writes its own.
pub(super) struct Fixture {
    pub(super) directory: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
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
    # The CLI prints the value with one record terminator; the backend
    # removes that terminator, never one the value carries.
    printf '\n'
    exit 0
  fi
  echo "secret $key not found" >&2
  exit 1
fi
if [ "$1" = "set" ]; then
  key="$2"
  if [ "$#" -eq 4 ] && [ "$3" = "--from-file" ]; then
    # `--from-file` is the byte-preserving store of the pinned CLI: the
    # store keeps the file's exact bytes, terminator and all.
    printf 'set\n' >> "$calls"
    mkdir -p "$store"
    cat "$4" > "$store/$key"
    exit 0
  fi
  [ "$#" -eq 2 ] || { echo "set takes the key and --from-file <file>" >&2; exit 9; }
  # The stdin store of the pinned CLI trims: '  padded value  ' is kept
  # as 'padded value', and the trailing newline goes with the trim, so a
  # value that must round-trip byte for byte never travels this path.
  printf 'set\n' >> "$calls"
  value=$(cat)
  value=$(printf '%s' "$value" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
  mkdir -p "$store"
  printf '%s' "$value" > "$store/$key"
  exit 0
fi
if [ "$1" = "env" ]; then
  [ "$#" -eq 3 ] && [ "$2" = "--json" ] && [ "$3" = "--describe" ] || { echo "env takes --json --describe" >&2; exit 7; }
  # The unfiltered query lists every key of the active profile with its
  # `env` and `as_file` flags: the store's own answer to whether a key is
  # a file-backed secret. The fixture's keys hold plain values.
  printf '{"schema":1,"keys":['
  separator=''
  if [ -d "$store" ]; then
    for file in "$store"/*; do
      [ -f "$file" ] || continue
      printf '%s{"key":"%s","kind":"secret","env":true,"as_file":false}' "$separator" "${file##*/}"
      separator=','
    done
  fi
  printf ']}'
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

    /// Writes a named executable script into the fixture directory, for
    /// store doubles the default script does not model.
    pub(super) fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write fixture script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("mode script");
        path
    }

    pub(super) fn resolver(&self) -> Resolver {
        Resolver::new(Arc::new(self.backend())).unwrap()
    }

    pub(super) fn set_calls(&self) -> usize {
        fs::read_to_string(self.directory.join("calls")).map_or(0, |calls| calls.lines().count())
    }

    pub(super) fn value(&self, key: &str) -> Option<String> {
        fs::read_to_string(self.directory.join("store").join(key)).ok()
    }

    pub(super) fn put(&self, key: &str, value: &str) {
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

pub(super) fn request(key: &str, format: &str) -> GeneratedRequest {
    GeneratedRequest {
        key: key.to_owned(),
        format: format.to_owned(),
    }
}

/// The twice-apply guarantee: the same workflow applied twice creates the
/// key exactly once. The first run generates, stores from a file, reads
/// back and compares; the second finds the key present and stores nothing.
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

/// The store round-trips the value through the real fixture: `set` reads
/// the value from a staging file via `--from-file`, never in an argument,
/// and the read back matches byte for byte.
#[test]
fn the_fixture_store_round_trips_from_file_values() {
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
    // any argument beside the key and `--from-file <file>`, so a run that
    // tried it would fail, not leak.
    assert_eq!(fixture.set_calls(), 3);
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
    let requests = generated_requests(&config).expect("collect");
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

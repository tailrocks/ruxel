//! Cache invalidation after a store attempt: a fetch that may have
//! changed a key never leaves the values cached before it, whatever the
//! attempt reported and whatever the read-back said.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A store that takes the write and then fails the read-back once: the
/// fetch stores the new value, the verification read errors, and a retry
/// of the same fetch finds the store already holding the content — the
/// sequence whose stale cache the invalidation must survive.
struct FailingReadback {
    gets: AtomicUsize,
    sets: AtomicUsize,
}
impl Backend for FailingReadback {
    fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
        match self.gets.fetch_add(1, Ordering::SeqCst) {
            // The values the run resolves before the fetch, and the
            // fetch's own comparison read.
            0..=2 => Ok(Some("synthetic-old-value".into())),
            // The read-back after the store: it fails.
            3 => Err("synthetic read-back failure".into()),
            // The retry's comparison read: the store holds the new value.
            _ => Ok(Some("synthetic-new-value".into())),
        }
    }
    fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
        self.sets.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn onepassword_fields(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> Result<serde_json::Value, String> {
        unreachable!("no 1Password fetch stores fnox")
    }
    fn onepassword_reference(&self, _: &str) -> Result<String, String> {
        unreachable!("no 1Password reference stores fnox")
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        unreachable!("no command lookup stores fnox")
    }
}

/// A fetch that fails its read-back still invalidates the cache: the
/// store attempt changed the key — the retry finds the new value and
/// takes the unchanged early return — so the values cached before the
/// attempt must not outlive it. A `Fnox` lookup and a `FnoxGenerated` one
/// read the same stored value, and neither answers the value the store
/// held before the fetch: a later resolution reads the store again.
#[test]
fn a_fetch_that_fails_its_read_back_still_invalidates_the_cache() {
    let backend = Arc::new(FailingReadback {
        gets: AtomicUsize::new(0),
        sets: AtomicUsize::new(0),
    });
    let resolver = Resolver::new(backend.clone()).expect("resolver");
    let plain = Lookup::Fnox {
        key: "FETCHED_KEY".into(),
    };
    let generated = Lookup::FnoxGenerated {
        key: "FETCHED_KEY".into(),
        format: "hex32".into(),
    };
    assert_eq!(
        resolver.resolve(&plain).expect("resolve"),
        "synthetic-old-value"
    );
    assert_eq!(
        resolver.resolve(&generated).expect("resolve"),
        "synthetic-old-value"
    );
    // The fetch stores the new value, and its read-back fails: the run
    // fails with the store already changed.
    assert!(
        resolver
            .fetch_secret("FETCHED_KEY", "synthetic-new-value")
            .is_err(),
        "the read-back failed"
    );
    assert_eq!(backend.sets.load(Ordering::SeqCst), 1, "one store write");
    // The retry of the same fetch finds the store holding the content —
    // the unchanged early return, which never stores and never verifies —
    // and the cached values are gone: both lookups read the store again.
    let (changed, _) = resolver
        .fetch_secret("FETCHED_KEY", "synthetic-new-value")
        .expect("the retry compares");
    assert!(!changed, "the store already holds the content");
    assert_eq!(backend.sets.load(Ordering::SeqCst), 1, "no second store");
    assert_eq!(
        resolver.resolve(&plain).expect("resolve"),
        "synthetic-new-value",
        "the plain lookup answered the value cached before the store"
    );
    assert_eq!(
        resolver.resolve(&generated).expect("resolve"),
        "synthetic-new-value",
        "the generated lookup answered the value cached before the store"
    );
}

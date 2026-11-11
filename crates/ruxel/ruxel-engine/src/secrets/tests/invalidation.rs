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

/// A store whose write hangs until the test releases it: the store
/// attempt is in flight while other lookups run, which is exactly the
/// window in which a cached read must not answer the old value.
struct RacingStore {
    value: Mutex<String>,
    store_started: Mutex<std::sync::mpsc::Sender<()>>,
    store_release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Backend for RacingStore {
    fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
        Ok(Some(self.value.lock().expect("value").clone()))
    }
    fn fnox_set(&self, _: &str, value: &str) -> Result<(), String> {
        self.store_started
            .lock()
            .expect("started")
            .send(())
            .expect("signal the start of the store");
        self.store_release
            .lock()
            .expect("release")
            .recv()
            .expect("wait for the store to be released");
        *self.value.lock().expect("value") = value.to_owned();
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

/// A read that races a store attempt on the same key must not answer
/// with the value cached before the attempt, and must not run before the
/// store's invalidation: the cached read and the store attempt serialize,
/// so the read waits for the invalidation and only then answers — the
/// value the store now holds.
#[test]
fn a_read_that_races_a_store_attempt_never_answers_the_old_value() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let resolver = Resolver::new(Arc::new(RacingStore {
        value: Mutex::new("racing-old-value".to_owned()),
        store_started: Mutex::new(started_tx),
        store_release: Mutex::new(release_rx),
    }))
    .expect("resolver");
    let lookup = Lookup::Fnox {
        key: "RACE_KEY".into(),
    };
    assert_eq!(
        resolver.resolve(&lookup).expect("resolve"),
        "racing-old-value",
        "the pre-fetch read caches the old value"
    );
    let fetcher = {
        let resolver = resolver.clone();
        std::thread::spawn(move || resolver.fetch_secret("RACE_KEY", "racing-new-value"))
    };
    started_rx.recv().expect("the store attempt is in flight");
    let (answer_tx, answer_rx) = std::sync::mpsc::channel();
    let reader = {
        let resolver = resolver.clone();
        let lookup = lookup.clone();
        std::thread::spawn(move || {
            answer_tx
                .send(resolver.resolve(&lookup))
                .expect("the racing read answers");
        })
    };
    let early = answer_rx.recv_timeout(std::time::Duration::from_millis(300));
    if let Ok(answer) = &early {
        assert_eq!(
            *answer,
            Ok("racing-new-value".to_owned()),
            "a read racing the store attempt answered with {answer:?}"
        );
    }
    release_tx.send(()).expect("release the store attempt");
    fetcher
        .join()
        .expect("the fetch thread finishes")
        .expect("the fetch stores");
    // The read is held until the store attempt and its invalidation are
    // done — the serialization the fix must provide — and then answers
    // with the value the store now holds.
    let answer = match early {
        Ok(answer) => answer,
        Err(_) => answer_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the racing read finishes once the store does"),
    };
    assert_eq!(
        answer,
        Ok("racing-new-value".to_owned()),
        "the racing read must see the stored value, never the old one: {answer:?}"
    );
    reader.join().expect("the reader thread finishes");
}

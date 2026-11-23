//! The races the generation preflight must survive: a key that appears
//! between the presence check and the store is never overwritten, and a
//! read-back that differs from what was stored fails the run.

use super::super::super::{Backend, Resolver};
use super::request;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A key that appears between the presence check and the store fails the
/// run rather than being overwritten: the store it appeared in keeps its
/// value. This is the defence in depth against a writer that does not take
/// the fnox write lock; a writer that arrives after the re-check is not
/// caught.
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

//! The store's trailing newlines: a value that ends in one round-trips,
//! and a value the store keeps in another form than it was given is
//! caught by the read-back comparison.

use super::generation::Fixture;
use super::*;
use std::sync::Arc;

/// A value that ends in a newline round-trips through the store: the
/// fetch that stores it reports a change, the next fetch of the same
/// content reports none, and a fetch whose content lost the final newline
/// is a change, not an equality. The store keeps the bytes it was given;
/// only the terminator the CLI adds when it prints a value is removed on
/// the way back.
#[test]
fn a_value_that_ends_in_a_newline_round_trips_and_a_missing_one_is_a_change() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    for (key, content) in [
        ("FETCHED_LF", "kubeconfig content\n"),
        ("FETCHED_CRLF", "kubeconfig content\r\n"),
    ] {
        // The first fetch stores the content and reports the change.
        let (changed, _) = resolver.fetch_secret(key, content).expect("store");
        assert!(changed, "{key}: the first fetch stores the content");
        // The same content again: the store holds it, so there is no change.
        let (changed, _) = resolver.fetch_secret(key, content).expect("compare");
        assert!(!changed, "{key}: the stored value round-trips");
        // The content without its final newline is another value.
        let shorter = content.trim_end_matches(['\r', '\n']);
        let (changed, _) = resolver.fetch_secret(key, shorter).expect("compare");
        assert!(changed, "{key}: a missing final newline is a change");
        // The store holds the bytes it was given, newline and all.
        assert_eq!(fixture.value(key).as_deref(), Some(shorter));
        // And the newline is a change again, in the other direction.
        let (changed, _) = resolver.fetch_secret(key, content).expect("compare");
        assert!(changed, "{key}: the newline is a change again");
        assert_eq!(fixture.value(key).as_deref(), Some(content));
    }
}

/// The write transport is byte-preserving: the pinned fnox trims the
/// whitespace around a value it reads from stdin — `'  padded value  \n'`
/// is stored as `padded value` — so the value travels in a staging file
/// and `fnox set KEY --from-file` reads the file's exact bytes. A value
/// with surrounding whitespace — and one that is only whitespace —
/// round-trips byte for byte, and the read-back comparison catches a
/// transport that trims. A value whose whitespace is inner, which no trim
/// touches, is the control.
#[test]
fn surrounding_whitespace_round_trips_through_the_store_transport() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    for (key, content) in [
        ("PADDED", "  padded value  \n"),
        ("SPACES_ONLY", "   \n"),
        ("INNER", "value  with  inner  spaces\n"),
    ] {
        let (changed, _) = resolver.fetch_secret(key, content).expect("store");
        assert!(changed, "{key}: the first fetch stores the content");
        assert_eq!(
            fixture.value(key).as_deref(),
            Some(content),
            "{key}: the store kept another form than the bytes it was given"
        );
        let (changed, _) = resolver.fetch_secret(key, content).expect("read back");
        assert!(!changed, "{key}: the stored value round-trips exactly");
    }
}

/// A store that keeps the value in another form than it was given — the
/// fnox CLI trims the whitespace around the value it reads from stdin — is
/// caught by the read-back comparison: the run fails rather than reporting
/// a value as stored that the store never held. Stripping a newline from
/// both sides would hide the difference.
#[test]
fn a_store_that_keeps_another_form_fails_the_run() {
    struct Trimming {
        stored: std::sync::Mutex<Option<String>>,
    }
    impl Backend for Trimming {
        fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
            Ok(self.stored.lock().map_err(|_| "poisoned store")?.clone())
        }
        fn fnox_set(&self, _: &str, value: &str) -> Result<(), String> {
            *self.stored.lock().map_err(|_| "poisoned store")? = Some(value.trim().to_owned());
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
    let resolver = Resolver::new(Arc::new(Trimming {
        stored: std::sync::Mutex::new(None),
    }))
    .unwrap();
    let error = resolver
        .fetch_secret("FETCHED_KEY", "kubeconfig content\n")
        .expect_err("the read-back differs from what was written");
    assert!(
        error.contains("read back from fnox differs"),
        "names the comparison: {error}"
    );
    assert!(!error.contains("kubeconfig"), "errors never carry values");
}

/// A value that ends in a bare CR round-trips through the store transport
/// byte for byte: the record terminator the CLI appends is one `\n`, so
/// the `\r` before it belongs to the value and must survive the read.
/// The store keeps the bytes it was given — CR and all — and the read
/// back that verifies the store finds them unchanged.
#[test]
fn a_value_that_ends_in_a_bare_cr_round_trips_through_the_store_transport() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    let (changed, _) = resolver.fetch_secret("BARE_CR", "value\r").expect("store");
    assert!(changed, "the first fetch stores the content");
    assert_eq!(
        fixture.value("BARE_CR").as_deref(),
        Some("value\r"),
        "the store kept the bytes it was given, CR and all"
    );
    let (changed, _) = resolver
        .fetch_secret("BARE_CR", "value\r")
        .expect("compare");
    assert!(
        !changed,
        "the stored value round-trips byte for byte, CR and all"
    );
}

/// The comparison distinguishes `value` from `value\r`: the CR belongs to
/// the value, not to the record terminator, so a store that holds one is
/// a change when the fetch carries the other — in both directions, never
/// an equality.
#[test]
fn the_comparison_distinguishes_a_bare_cr_value_from_one_without() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    for (held, fetched) in [("value\r", "value"), ("value", "value\r")] {
        fixture.put("BARE_CR_CHANGE", held);
        let (changed, _) = resolver
            .fetch_secret("BARE_CR_CHANGE", fetched)
            .expect("compare");
        assert!(
            changed,
            "the store holds {held:?}, the fetch of {fetched:?} is a change"
        );
        assert_eq!(
            fixture.value("BARE_CR_CHANGE").as_deref(),
            Some(fetched),
            "the fetch replaces the stored value"
        );
    }
}

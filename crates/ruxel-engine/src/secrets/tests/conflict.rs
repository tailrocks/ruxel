//! One key, one format: the same key referenced with two formats is a
//! configuration error, not a choice made by the order the configuration is
//! walked in.

use super::generation::{Fixture, request};
use super::*;
use serde_json::json;

/// One key is created in one format: the same key referenced with two
/// formats is a configuration error, not a choice made by the order the
/// configuration is walked in. The error names the key and both formats,
/// whichever order the references come in.
#[test]
fn a_key_referenced_with_two_formats_is_a_configuration_error() {
    for formats in [["hex32", "alnum32"], ["alnum32", "hex32"]] {
        let config = json!({
            "plays": [{"tasks": [
                {"operation": {"kind": "copy", "content": {"kind": "concat", "args": [
                    {"kind": "lookup", "provider": "fnox-generated", "key": "PROCESSOR_DB_PASSWORD", "format": formats[0]},
                    {"kind": "lookup", "provider": "fnox-generated", "key": "PROCESSOR_DB_PASSWORD", "format": formats[1]}
                ]}}}
            ]}]
        });
        let error = generated_requests(&config).expect_err("one key, one format");
        assert!(
            error.contains("PROCESSOR_DB_PASSWORD"),
            "names the key: {error}"
        );
        assert!(error.contains("alnum32"), "names both formats: {error}");
        assert!(error.contains("hex32"), "names both formats: {error}");
    }
}

/// The same conflict fails the run before any key is created: the store
/// keeps no value for a key the configuration names in two formats.
#[test]
fn a_conflicting_format_creates_nothing() {
    let fixture = Fixture::new();
    let resolver = fixture.resolver();
    let error = resolver
        .ensure_generated(
            &[
                request("PROCESSOR_DB_PASSWORD", "hex32"),
                request("PROCESSOR_DB_PASSWORD", "alnum32"),
            ],
            &mut |_, _, _| {},
        )
        .expect_err("one key, one format");
    assert!(
        error.contains("PROCESSOR_DB_PASSWORD")
            && error.contains("alnum32")
            && error.contains("hex32"),
        "names the key and both formats: {error}"
    );
    assert_eq!(fixture.set_calls(), 0, "no key was created");
}

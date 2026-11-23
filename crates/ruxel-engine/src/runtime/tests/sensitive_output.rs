//! The sensitive event projection: what a censored run keeps.

use super::*;

/// A task object in an event is a plan projection, and the
/// known-after-apply marker it carries is controller-built metadata the
/// plan keeps through its own conservative projection: the sensitive run
/// keeps it too, and nothing else of the task's fields.
#[test]
fn sensitive_event_keeps_the_known_after_apply_marker_of_a_task_object() -> Result<()> {
    let projected = output::sensitive_event(&json!({
        "host":"fixture",
        "task":{"name":"synthetic-marker","noLog":true,"censored":true,"knownAfterApply":true},
        "planned":true
    }));
    assert_eq!(
        projected["task"],
        json!({"censored":true,"knownAfterApply":true})
    );
    assert!(!serde_json::to_string(&projected)?.contains("synthetic-marker"));
    // A task object without the marker is censored whole.
    let unmarked = output::sensitive_event(&json!({
        "task":{"name":"synthetic-marker","censored":true}
    }));
    assert_eq!(unmarked["task"], json!({"censored":true}));
    Ok(())
}

/// The fingerprint and the diff sentence a sensitive run keeps are the
/// controller's own: exactly eight lowercase hex, and the sentence built
/// over them, whole. An agent-supplied fingerprint — any other string —
/// and a diff that merely starts like the sentence and carries material
/// after it are dropped, so agent material reaches no censored output.
#[test]
fn sensitive_event_passes_only_controller_built_fingerprints_and_diffs() -> Result<()> {
    let projected = output::sensitive_event(&json!({
        "host":"fixture",
        "result":{
            "changed":true,
            "fingerprint":"synthetic-agent-fingerprint",
            "diff":"content changed (fingerprint abcd1234) synthetic-agent-material"
        }
    }));
    assert!(
        projected["result"].get("fingerprint").is_none(),
        "{projected}"
    );
    assert!(projected["result"].get("diff").is_none(), "{projected}");
    assert!(!serde_json::to_string(&projected)?.contains("synthetic-agent"));
    // The controller's own eight-hex fingerprint and the exact sentence
    // built over it stay.
    let kept = output::sensitive_event(&json!({
        "host":"fixture",
        "result":{
            "changed":true,
            "fingerprint":"abcd1234",
            "diff":"stored in fnox (fingerprint abcd1234)"
        }
    }));
    assert_eq!(kept["result"]["fingerprint"], json!("abcd1234"));
    assert_eq!(
        kept["result"]["diff"],
        json!("stored in fnox (fingerprint abcd1234)")
    );
    Ok(())
}

/// A diff that carries the controller's exact shape is not the
/// controller's: `73656372` is the hex of `secr`, four bytes of secret in
/// the fingerprint's own shape, and a projection that accepts the
/// sentence on shape alone projects them. The sentence a sensitive run
/// keeps is the controller's own — the one built over the fingerprint the
/// controller set beside it — so a result with no controller fingerprint
/// keeps no diff, and a result whose diff names another fingerprint keeps
/// neither the sentence nor the claim that it was built here.
#[test]
fn a_valid_shaped_sentence_alone_is_not_the_controllers() -> Result<()> {
    // The sentence with no fingerprint beside it: nothing ties it to a
    // fingerprint the controller computed, so it is censored.
    let sentence_only = output::sensitive_event(&json!({
        "host": "fixture",
        "result": {
            "changed": true,
            "diff": "content changed (fingerprint 73656372)"
        }
    }));
    assert!(
        sentence_only["result"].get("diff").is_none(),
        "{sentence_only}"
    );
    // The sentence beside a fingerprint it is not built over: the
    // fingerprint keeps (it is the controller's shape over its own key),
    // the sentence goes — it names four bytes the controller never
    // fingerprinted.
    let mismatched = output::sensitive_event(&json!({
        "host": "fixture",
        "result": {
            "changed": true,
            "fingerprint": "abcd1234",
            "diff": "content changed (fingerprint 73656372)"
        }
    }));
    assert_eq!(mismatched["result"]["fingerprint"], json!("abcd1234"));
    assert!(
        mismatched["result"].get("diff").is_none(),
        "a sentence over another fingerprint passed: {mismatched}"
    );
    assert!(
        !serde_json::to_string(&mismatched)?.contains("73656372"),
        "four bytes of agent material reached the projection: {mismatched}"
    );
    Ok(())
}

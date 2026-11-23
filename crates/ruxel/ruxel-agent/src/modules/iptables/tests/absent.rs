use super::{context, run_with, scratch, stub};
use serde_json::json;

#[test]
fn state_absent_deletes_a_present_rule_and_is_idempotent_when_absent() {
    let dir = scratch("absent");
    let (binary, marker, log) = stub(&dir, true);
    let params = json!({"chain":"INPUT", "protocol":"tcp", "jump":"DROP", "state":"absent"});
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert!(!marker.exists(), "the rule is gone");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains("-D INPUT"), "{calls}");
    // The rule is not there any more: removing it again changes nothing.
    let again = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(again["changed"], false);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_mode_absent_mutates_nothing() {
    let dir = scratch("check");
    let (binary, marker, log) = stub(&dir, true);
    let params = json!({"chain":"INPUT", "protocol":"tcp", "jump":"DROP", "state":"absent"});
    let before = std::fs::read_to_string(&log).unwrap_or_default();
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(true),
    )
    .unwrap();
    // Check mode reports the pending change and touches nothing.
    assert_eq!(result["changed"], true);
    assert!(marker.exists(), "the rule is still there");
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    let added = calls.len() - before.len();
    // Check mode asks the same question the removal asks: is there a rule
    // of the chain that matches? It lists and it deletes nothing.
    assert!(calls[before.len()..].contains("-S INPUT"), "{calls}");
    assert!(!calls[before.len()..].contains("-D"), "{calls}");
    assert!(!calls[before.len()..].contains("-A"), "{calls}");
    assert!(added > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

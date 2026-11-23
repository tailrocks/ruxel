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

/// A stub that keeps the rules of the chain, one per line, the way the
/// firewall lists them, and deletes the rule it is told to delete.
fn listing_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn numeric_protocol_meets_the_listed_name_on_removal() {
    let dir = scratch("absent-numeric-protocol");
    // The firewall stores 6 and prints `tcp`: an absent rule that names
    // the number still names the listed rule.
    let (binary, file) = listing_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"6", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_bare_chain_line_is_the_empty_rule() {
    let dir = scratch("absent-chain-only");
    // A rule with no arguments is listed as exactly `-A CHAIN`, with no
    // trailing space: a chain-only absent rule still names it.
    let file = dir.join("rules");
    std::fs::write(&file, "-P INPUT ACCEPT\n-A INPUT\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) cat '{file}';;\n  -D) echo \"$*\" > '{dir}/deleted'; grep -v -F -x -- \"-A INPUT\" '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\nesac\nexit 0\n",
            file = file.display(),
            dir = dir.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let params = json!({"chain":"INPUT", "state":"absent"});
    let result = run_with(
        script.to_str().unwrap(),
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert!(dir.join("deleted").exists(), "the bare rule is deleted");
    std::fs::remove_dir_all(dir).unwrap();
}

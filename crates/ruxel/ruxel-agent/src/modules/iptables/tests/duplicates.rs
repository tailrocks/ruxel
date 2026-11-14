use super::super::{delete_matching, rule_spec};
use super::*;
use serde_json::json;

/// A stub that keeps the rules of the chain, one per line, and answers
/// the listing the way the firewall does.
fn listing_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -D) sed -i.bak \"${{3}}d\" '{file}' && rm -f '{file}.bak'; exit 0;;\n  -C) exit 1;;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn state_absent_deletes_every_rule_that_matches_the_same_way() {
    let dir = scratch("matches");
    // Two rules with the same match and different jumps: the chain and
    // the match are the rule's identity, so both go.
    let (binary, file) = listing_stub(
        &dir,
        &[
            "-p tcp -d 10.0.0.0/8 -j DROP",
            "-p tcp -d 10.0.0.0/8 -j ACCEPT",
            "-p udp -d 10.0.0.0/8 -j DROP",
        ],
    );
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
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
    assert_eq!(left, "-p udp -d 10.0.0.0/8 -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_deletion_bound_is_read_before_the_next_deletion() {
    let dir = scratch("bound");
    let (binary, file) = listing_stub(
        &dir,
        &[
            "-p tcp -d 10.0.0.0/8 -j DROP",
            "-p tcp -d 10.0.0.0/8 -j ACCEPT",
            "-p tcp -d 10.0.0.0/8 -j REJECT",
        ],
    );
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"absent"
    });
    let spec = rule_spec(params.as_object().unwrap());
    // A bound below the number of rules leaves the rule, and the removal
    // says so.
    let error = delete_matching(&binary, "INPUT", &spec, 1).unwrap_err();
    assert!(error.contains("still there"), "{error}");
    // A bound that covers every rule empties the chain and says nothing.
    // The first call took one of the three; the second takes the rest.
    assert_eq!(delete_matching(&binary, "INPUT", &spec, 5).unwrap(), 2);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_mode_asks_the_same_question_the_removal_answers() {
    let dir = scratch("check-mode");
    // The rule of the chain matches the declared match and has another
    // jump: check mode says there is a change, like the removal does.
    let (binary, _file) = listing_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j ACCEPT"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(true),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    std::fs::remove_dir_all(dir).unwrap();
}

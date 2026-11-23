use super::*;
use serde_json::json;

/// A stub that keeps the rules of the chain, one per line, and answers
/// the listing the way the firewall does. It appends the rule it is
/// given, deletes the rule it is told to delete, and answers the probe
/// with the rule it holds.
fn chain_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\n  -A) shift 2; echo \"$*\" >> '{file}';;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn present_mode_takes_the_stale_jump_of_the_match_away() {
    let dir = scratch("present");
    // The chain holds the declared match with another jump: it ends the
    // match before the declared rule is ever read, so it shadows the
    // declared rule and must go before it is installed.
    let (binary, file) = chain_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j ACCEPT"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"present"
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
    assert_eq!(left, "-p tcp -d 10.0.0.0/8 -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn present_mode_keeps_the_rule_it_installed() {
    let dir = scratch("present-again");
    // The declared rule is the rule of the chain: nothing changes, and
    // the rule is not installed a second time.
    let (binary, file) = chain_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"present"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], false);
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -d 10.0.0.0/8 -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_mode_reports_the_stale_jump_and_touches_nothing() {
    let dir = scratch("present-check");
    let (binary, file) = chain_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j ACCEPT"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"present"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(true),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -d 10.0.0.0/8 -j ACCEPT\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_commented_rule_the_firewall_lists_is_already_there() {
    let dir = scratch("comment-converges");
    // The firewall prints the match info before the target: a declared
    // commented rule the chain already holds is there, not stale, so a
    // reapply changes nothing and never deletes the only rule.
    let (binary, file) = chain_stub(&dir, &["-p tcp -m comment --comment hello -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "jump":"DROP", "comment":"hello",
        "state":"present"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], false);
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -m comment --comment hello -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

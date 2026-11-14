#[path = "tests/duplicates.rs"]
mod duplicates;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

#[test]
fn probe_distinguishes_absent_rule_from_command_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-probe-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let executable = dir.join("iptables-stub");
    std::fs::write(&executable, "#!/bin/sh\ntest \"$LC_ALL\" = C || exit 9\ncase $1 in\n present) exit 0;;\n missing) echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n denied) echo 'Permission denied' >&2; exit 1;;\n invalid) echo 'unknown option' >&2; exit 2;;\nesac\nexit 7\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = executable.to_str().unwrap();
    assert!(super::probe(binary, &["present"]).unwrap());
    assert!(!super::probe(binary, &["missing"]).unwrap());
    assert!(super::probe(binary, &["denied"]).is_err());
    assert!(super::probe(binary, &["invalid"]).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn builds_rule_spec_and_parses_policy() {
    let params =
        json!({"protocol":"tcp", "destination":"10.0.0.0/8", "jump":"ACCEPT", "comment":"allow"});
    let spec = super::rule_spec(params.as_object().unwrap());
    assert_eq!(
        spec,
        [
            "-p",
            "tcp",
            "-d",
            "10.0.0.0/8",
            "-j",
            "ACCEPT",
            "-m",
            "comment",
            "--comment",
            "allow"
        ]
    );
    assert_eq!(
        super::parse_policy("-P INPUT DROP\n-A INPUT -j ACCEPT\n", "INPUT").as_deref(),
        Some("DROP")
    );
}

use super::{ExecContext, run_with};

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    }
}

/// A stub that stands in for iptables: the rule is there when the marker
/// file is, and every call is written to the log.
fn stub(dir: &std::path::Path, present: bool) -> (String, std::path::PathBuf, std::path::PathBuf) {
    let marker = dir.join("rule-present");
    if present {
        std::fs::write(&marker, "rule\n").unwrap();
    } else {
        let _ = std::fs::remove_file(&marker);
    }
    let log = dir.join("calls.log");
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in\n  -S) if [ -f '{marker}' ]; then echo \"-P $2 ACCEPT\"; echo \"-A $2 -p tcp -j DROP\"; fi; exit 0;;\n  -C) if [ -f '{marker}' ]; then exit 0; else echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1; fi;;\n  -D) rm -f '{marker}'; exit 0;;\n  -A) touch '{marker}'; exit 0;;\nesac\nexit 0\n",
            log = log.display(),
            marker = marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), marker, log)
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
    assert!(calls[before.len()..].contains("-C INPUT"), "{calls}");
    assert!(!calls[before.len()..].contains("-D"), "{calls}");
    assert!(!calls[before.len()..].contains("-A"), "{calls}");
    assert!(added > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

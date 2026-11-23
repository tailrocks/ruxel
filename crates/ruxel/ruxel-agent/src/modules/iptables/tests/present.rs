use super::*;
use serde_json::json;

/// A stub that keeps the rules of the chain, one per line, and answers
/// the listing the way the firewall does. It appends the rule it is
/// given, deletes the rule it is told to delete, answers the probe
/// with the rule it holds, and commits a restore the way
/// `iptables-restore` does: the `-A` lines it reads become the chain.
/// The restore sibling is a link to this script: it reads the commit
/// from standard input instead of the command line.
fn chain_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$(basename \"$0\")\" in\n  *-restore) tmp='{file}.new'; : > \"$tmp\"; while IFS= read -r line; do case \"$line\" in -A\\ *) rest=\"${{line#-A }}\"; echo \"${{rest#* }}\" >> \"$tmp\";; esac; done; mv \"$tmp\" '{file}'; exit 0;;\nesac\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\n  -A) shift 2; echo \"$*\" >> '{file}';;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&script, dir.join("iptables-stub-restore")).unwrap();
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
fn present_mode_replaces_every_stale_rule_in_one_transaction() {
    let dir = scratch("present-atomic");
    // Two stale rules order the packets around an unrelated rule: the
    // replacement must take the first stale position in a single commit,
    // so the packets never cross a half-replaced chain.
    let (binary, file) = chain_stub(
        &dir,
        &[
            "-p tcp -d 10.0.0.0/8 -j ACCEPT",
            "-p udp -j ACCEPT",
            "-p tcp -d 10.0.0.0/8 -j REJECT --reject-with tcp-reset",
        ],
    );
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
    assert_eq!(
        left, "-p tcp -d 10.0.0.0/8 -j DROP\n-p udp -j ACCEPT\n",
        "{left}"
    );
    // Converged: running again changes nothing.
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

/// A stub whose restore holds the lock twice: the first two commits
/// fail the way a competing writer makes them fail, and the third goes
/// through.
fn contended_stub(dir: &std::path::Path) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, "-p tcp -d 10.0.0.0/8 -j ACCEPT\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$(basename \"$0\")\" in\n  *-restore) count=$(cat '{dir}/attempts' 2>/dev/null || echo 0); count=$((count + 1)); echo \"$count\" > '{dir}/attempts'; if [ \"$count\" -le 2 ]; then echo 'iptables-restore: Resource temporarily unavailable' >&2; exit 4; fi; tmp='{file}.new'; : > \"$tmp\"; while IFS= read -r line; do case \"$line\" in -A\\ *) rest=\"${{line#-A }}\"; echo \"${{rest#* }}\" >> \"$tmp\";; esac; done; mv \"$tmp\" '{file}'; exit 0;;\nesac\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\nesac\nexit 0\n",
            file = file.display(),
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&script, dir.join("iptables-stub-restore")).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn present_mode_retries_a_restore_behind_the_lock() {
    let dir = scratch("present-retry");
    let (binary, file) = contended_stub(&dir);
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
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "-p tcp -d 10.0.0.0/8 -j DROP\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A stub whose restore commits and then lets a concurrent writer win:
/// every commit carries the stale rule back, so the chain never holds
/// only the declared rule.
fn poisoned_stub(dir: &std::path::Path) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, "-p tcp -d 10.0.0.0/8 -j ACCEPT\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$(basename \"$0\")\" in\n  *-restore) tmp='{file}.new'; : > \"$tmp\"; while IFS= read -r line; do case \"$line\" in -A\\ *) rest=\"${{line#-A }}\"; echo \"${{rest#* }}\" >> \"$tmp\";; esac; done; echo '-p tcp -d 10.0.0.0/8 -j ACCEPT' >> \"$tmp\"; mv \"$tmp\" '{file}'; exit 0;;\nesac\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&script, dir.join("iptables-stub-restore")).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn present_mode_fails_instead_of_reporting_a_rule_it_did_not_install() {
    let dir = scratch("present-poison");
    // A writer carries the stale jump back after every commit: the apply
    // lists the chain again after each restore, finds it unconverged and
    // fails instead of reporting the declared rule installed.
    let (binary, _file) = poisoned_stub(&dir);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.0/8", "jump":"DROP",
        "state":"present"
    });
    let error = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap_err();
    assert!(error.contains("did not converge"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}

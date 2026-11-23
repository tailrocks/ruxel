use super::{context, run_with};
use std::os::unix::fs::PermissionsExt;

/// A stub that lists the rules of the chain, inserts at a position,
/// appends, and deletes, logging every mutation in order.
fn position_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let log = dir.join("log");
    std::fs::write(&log, "").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) echo \"D $*\" >> '{log}'; shift 2; awk -v n=\"$*\" 'BEGIN{{done=0}} {{if (!done && $0 == n) {{done=1; next}} print}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\n  -A) echo \"A $*\" >> '{log}'; shift 2; echo \"$*\" >> '{file}';;\n  -I) echo \"I $*\" >> '{log}'; chain=\"$2\"; at=\"$3\"; shift 3; awk -v n=\"$*\" -v p=\"$at\" 'NR==p{{print n}} {{print}} END{{if (NR<p) print n}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\nesac\nexit 0\n",
            file = file.display(),
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

fn fresh_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn params(obj: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    obj.as_object().unwrap().clone()
}

#[test]
fn a_replacement_lands_where_the_stale_rule_stood() {
    // An early ACCEPT with the declared match is replaced by DROP
    // while a later catch-all ACCEPT matches too: the DROP must land
    // where the stale rule stood, ahead of the catch-all, and the
    // stale rule must go.
    let dir = fresh_dir("position");
    let (binary, file) = position_stub(
        &dir,
        &["-p tcp -d 10.0.0.0/8 -j ACCEPT", "-p tcp -j ACCEPT"],
    );
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "present"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "-p tcp -d 10.0.0.0/8 -j DROP\n-p tcp -j ACCEPT\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_replacement_is_installed_before_any_stale_rule_goes() {
    // The chain holds two stale rules for the match: the
    // replacement is installed first, so no deletion leaves a
    // window where a stale rule answers alone.
    let dir = fresh_dir("ordering");
    let (binary, _) = position_stub(
        &dir,
        &[
            "-p tcp -d 10.0.0.0/8 -j ACCEPT",
            "-p tcp -d 10.0.0.0/8 -j REJECT",
        ],
    );
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "present"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    let log = std::fs::read_to_string(dir.join("log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert!(lines[0].starts_with("I -I INPUT 1 "), "{log}");
    assert!(
        lines.iter().skip(1).all(|line| line.starts_with("D -D ")),
        "{log}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_concurrent_writer_does_not_leave_the_match_unfiltered() {
    // Another writer takes the declared rule after the probe and
    // leaves a stale same-match rule: the run must converge on the
    // declared rule instead of reporting success with it gone.
    let dir = fresh_dir("converge");
    let file = dir.join("rules");
    std::fs::write(
        &file,
        "-p tcp -d 10.0.0.0/8 -j DROP\n-p tcp -d 10.0.0.0/8 -j ACCEPT\n",
    )
    .unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; awk -v n=\"$*\" 'BEGIN{{done=0}} {{if (!done && $0 == n) {{done=1; next}} print}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}'; grep -v -F -x -e '-p tcp -d 10.0.0.0/8 -j DROP' '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\n  -A) shift 2; echo \"$*\" >> '{file}';;\n  -I) chain=\"$2\"; at=\"$3\"; shift 3; awk -v n=\"$*\" -v p=\"$at\" 'NR==p{{print n}} {{print}} END{{if (NR<p) print n}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = script.to_str().unwrap().to_owned();
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "present"
        })),
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

#[test]
fn a_reject_with_other_options_is_stale_but_its_default_is_not() {
    // A REJECT that answers with another type is not the declared
    // plain REJECT: it goes, and the declared one is installed.
    let dir = fresh_dir("reject-opts");
    let (binary, file) = position_stub(&dir, &["-p tcp -j REJECT --reject-with tcp-reset"]);
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp",
            "jump": "REJECT", "state": "present"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "-p tcp -j REJECT\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_serialized_reject_default_is_the_declared_rule() {
    // The firewall prints the default reject type even when the
    // declaration omits it: that serialization is the declared
    // rule, not a stale one, so the run changes nothing.
    let dir = fresh_dir("reject-default");
    let (binary, file) = position_stub(
        &dir,
        &["-p tcp -j REJECT --reject-with icmp-port-unreachable"],
    );
    let stub_c = std::fs::read_to_string(dir.join("iptables-stub")).unwrap();
    // The probe must see the rule the declaration names: teach the
    // stub that `-C` with omitted target options matches the
    // serialized default.
    std::fs::write(
        dir.join("iptables-stub"),
        stub_c.replace(
            "-C) shift 2; if grep -q -F -x -e \"$*\" '",
            "-C) shift 2; if [ \"$*\" = \"-p tcp -j REJECT\" ]; then exit 0; fi; if grep -q -F -x -e \"$*\" '",
        ),
    )
    .unwrap();
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp",
            "jump": "REJECT", "state": "present"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "-p tcp -j REJECT --reject-with icmp-port-unreachable\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_rule_whose_comment_names_a_jump_is_the_declared_rule() {
    // The comment is `-j`: the declared rule is already there, so
    // the run changes nothing and, above all, does not delete the
    // only rule that filters the match. The listed line is in the
    // order the declaration uses: the stub matches the probe by
    // exact string, while the real firewall parses the arguments.
    let dir = fresh_dir("comment-jump");
    let (binary, file) = position_stub(&dir, &["-p tcp -j DROP -m comment --comment -j"]);
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp",
            "jump": "DROP", "comment": "-j", "state": "present"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "-p tcp -j DROP -m comment --comment -j\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn absent_finds_a_zoned_destination_without_its_zone() {
    let dir = fresh_dir("zone-absent");
    let (binary, file) = position_stub(&dir, &["-d fe80::1/128 -j DROP"]);
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "destination": "fe80::1%eth0",
            "jump": "DROP", "state": "absent", "ip_version": "ipv6"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn absent_finds_protocol_zero_in_a_rule_without_protocol() {
    let dir = fresh_dir("proto-zero");
    let (binary, file) = position_stub(&dir, &["-j ACCEPT"]);
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "0",
            "jump": "ACCEPT", "state": "absent"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn absent_finds_a_match_handed_to_another_chain() {
    let dir = fresh_dir("goto-absent");
    let (binary, file) = position_stub(&dir, &["-p tcp -g ALLOW_CHAIN"]);
    let result = run_with(
        &binary,
        "INPUT",
        &params(&serde_json::json!({
            "chain": "INPUT", "protocol": "tcp",
            "jump": "DROP", "state": "absent"
        })),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

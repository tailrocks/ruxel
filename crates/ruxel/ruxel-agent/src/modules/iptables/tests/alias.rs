use super::super::super::ExecContext;
use super::super::run_with;
use std::os::unix::fs::PermissionsExt;

/// A stub that lists the rules of the chain and deletes the exact
/// line it is told to delete.
fn listing_stub(dir: &std::path::Path, rule: &str) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, format!("{rule}\n")).unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\nesac\nexit 0\n",
            file = file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), file)
}

#[test]
fn absent_mode_finds_the_rule_by_its_printed_protocol_name() {
    // `ip6tables` reads `icmpv6` as protocol 58 but prints the first
    // name of that number, `ipv6-icmp`: the rule Ruxel installed is
    // the listed rule, and absent mode must take it away.
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-alias-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (binary, file) = listing_stub(&dir, "-p ipv6-icmp -j DROP");
    let params = serde_json::json!({
        "chain": "INPUT", "protocol": "icmpv6", "jump": "DROP", "state": "absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        },
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn absent_mode_ignores_the_target_options_of_a_reject_rule() {
    // The firewall serializes the REJECT target with `--reject-with`
    // even for the default type: those target options are not packet
    // matchers, and the listed rule is the declared one.
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-reject-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (binary, file) = listing_stub(&dir, "-p tcp -j REJECT --reject-with icmp-net-unreachable");
    let params = serde_json::json!({
        "chain": "INPUT", "protocol": "tcp", "jump": "REJECT", "state": "absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        },
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

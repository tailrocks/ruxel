use serde_json::json;

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

use super::*;

#[test]
fn state_absent_deletes_every_duplicate_of_the_rule() {
    let dir = scratch("duplicates");
    // A stub that holds a count of identical rules: `iptables -D` takes
    // one matching rule at a time, so a chain with two copies needs two
    // deletions.
    let marker = dir.join("rule-count");
    std::fs::write(&marker, "2\n").unwrap();
    let log = dir.join("calls.log");
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in\n  -C) n=$(cat '{marker}' 2>/dev/null || echo 0); if [ \"$n\" -gt 0 ]; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) n=$(cat '{marker}' 2>/dev/null || echo 0); if [ \"$n\" -gt 0 ]; then echo $((n-1)) > '{marker}'; fi; exit 0;;\nesac\nexit 0\n",
            log = log.display(),
            marker = marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = script.to_str().unwrap().to_owned();
    let params = json!({"chain":"INPUT", "protocol":"tcp", "jump":"DROP", "state":"absent"});
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "0",
        "every copy of the rule is gone"
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.matches("-D INPUT").count(), 2, "{calls}");
    std::fs::remove_dir_all(dir).unwrap();
}

use super::super::rules::{full_mask, match_pairs, rule_comment, rule_jump};

fn args(line: &[&str]) -> Vec<String> {
    line.iter().map(|item| (*item).to_string()).collect()
}

#[test]
fn a_comment_value_is_not_the_jump() {
    // The comment names `-j`: the jump is still the target behind
    // it, not the comment's own value.
    let listed = args(&[
        "-p",
        "tcp",
        "-m",
        "comment",
        "--comment",
        "-j",
        "-j",
        "DROP",
    ]);
    assert_eq!(rule_jump(&listed).as_deref(), Some("DROP"));
    assert_eq!(rule_comment(&listed).as_deref(), Some("-j"));
}

#[test]
fn a_zoned_destination_meets_the_listed_network() {
    // The firewall never prints the zone: a declared zone meets the
    // listed network without one.
    assert_eq!(full_mask("fe80::1%eth0/128").unwrap(), "fe80::1/128");
    assert_eq!(full_mask("fe80::1/128").unwrap(), "fe80::1/128");
    let declared = args(&["-d", "fe80::1%eth0/128", "-j", "DROP"]);
    let listed = args(&["-d", "fe80::1/128", "-j", "DROP"]);
    assert_eq!(match_pairs(&declared), match_pairs(&listed));
}

#[test]
fn protocol_zero_is_the_omitted_default() {
    // Protocol 0 is the wildcard the firewall omits from the
    // listing, the way it omits `all`.
    let declared = args(&["-p", "0", "-j", "DROP"]);
    let listed = args(&["-j", "DROP"]);
    assert_eq!(match_pairs(&declared), match_pairs(&listed));
}

#[test]
fn a_goto_ends_the_match_like_a_jump() {
    // A rule that hands the packet to a chain matches the declared
    // match: the goto is the target, not match data.
    let listed = args(&["-p", "tcp", "-g", "ALLOW_CHAIN"]);
    let declared = args(&["-p", "tcp", "-j", "DROP"]);
    assert_eq!(match_pairs(&listed), match_pairs(&declared));
    assert_eq!(rule_jump(&listed).as_deref(), Some("ALLOW_CHAIN"));
}

#[test]
fn numeric_protocol_aliases_meet_the_printed_names() {
    // The firewall prints the first name of the protocol number, so a
    // declared number meets the listed name.
    for (number, name) in [
        ("1", "icmp"),
        ("2", "igmp"),
        ("6", "tcp"),
        ("17", "udp"),
        ("47", "gre"),
        ("50", "esp"),
        ("51", "ah"),
        ("58", "ipv6-icmp"),
        ("132", "sctp"),
    ] {
        let declared = args(&["-p", number, "-j", "DROP"]);
        let listed = args(&["-p", name, "-j", "DROP"]);
        assert_eq!(match_pairs(&declared), match_pairs(&listed), "{number}");
    }
}

#[test]
fn builds_rule_spec_and_parses_policy() {
    let params = serde_json::json!({"protocol":"tcp", "destination":"10.0.0.0/8", "jump":"ACCEPT", "comment":"allow"});
    let spec = super::super::rules::rule_spec(params.as_object().unwrap()).unwrap();
    assert_eq!(
        spec,
        [
            "-p",
            "tcp",
            "-d",
            "10.0.0.0/8",
            "-m",
            "comment",
            "--comment",
            "allow",
            "-j",
            "ACCEPT"
        ]
    );
    assert_eq!(
        super::super::chain::parse_policy("-P INPUT DROP\n-A INPUT -j ACCEPT\n", "INPUT")
            .as_deref(),
        Some("DROP")
    );
}

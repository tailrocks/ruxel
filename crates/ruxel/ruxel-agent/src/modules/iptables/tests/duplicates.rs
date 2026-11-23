use super::super::chain::delete_matching;
use super::super::rules::{full_mask, match_pairs, rule_jump, rule_spec};
use super::*;
use serde_json::json;

/// One argument per item: the token lists the option tests hand to the
/// match and jump readers.
fn tokens(items: &[&str]) -> Vec<String> {
    items.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn an_ipv6_zone_is_canonicalized_away() {
    // The firewall stores the address and the mask but not the zone, so
    // the zone is dropped and the bare address is masked.
    assert_eq!(full_mask("fe80::1%eth0").unwrap(), "fe80::1/128");
    assert_eq!(full_mask("fe80::1%eth0/10").unwrap(), "fe80::/10");
    assert_eq!(full_mask("::1%lo").unwrap(), "::1/128");
}

#[test]
fn a_comment_value_is_not_the_jump() {
    // A comment that spells a jump flag is a value: the target is the
    // flag past it, in both spellings of the flag and the comment.
    assert_eq!(
        rule_jump(&tokens(&[
            "-p",
            "tcp",
            "-m",
            "comment",
            "--comment",
            "-j",
            "-j",
            "DROP"
        ]))
        .as_deref(),
        Some("DROP")
    );
    assert_eq!(
        rule_jump(&tokens(&["-p", "tcp", "--comment", "--jump", "-j", "DROP"])).as_deref(),
        Some("DROP")
    );
}

#[test]
fn a_goto_ends_the_match() {
    // `-g` hands the packet to another chain the way `-j` does: what
    // follows is the target's own, in both spellings.
    assert_eq!(
        match_pairs(&tokens(&["-p", "tcp", "-g", "ALLOW"])),
        vec![("-p".to_string(), "tcp".to_string())]
    );
    assert_eq!(
        match_pairs(&tokens(&["-p", "tcp", "--goto", "ALLOW"])),
        vec![("-p".to_string(), "tcp".to_string())]
    );
}

#[test]
fn protocol_zero_matches_the_omitted_default() {
    // The firewall reads protocol `0` as `all` and prints no `-p` for
    // it, so neither spelling is part of the match.
    assert_eq!(match_pairs(&tokens(&["-p", "0", "-j", "DROP"])), Vec::new());
    assert_eq!(
        match_pairs(&tokens(&["-p", "all", "-j", "DROP"])),
        Vec::new()
    );
}

#[test]
fn normalizes_destinations_and_refuses_malformed_ones() {
    // The destination in the form the firewall prints it: a host without
    // a prefix carries its full mask, and the host bits of a prefix are
    // masked off.
    assert_eq!(full_mask("10.0.0.1").unwrap(), "10.0.0.1/32");
    assert_eq!(full_mask("::1").unwrap(), "::1/128");
    assert_eq!(full_mask("10.0.0.1/8").unwrap(), "10.0.0.0/8");
    assert_eq!(full_mask("2001:db8::1/32").unwrap(), "2001:db8::/32");
    // A complete CIDR comes before any normalization: a prefix that is
    // not a number must not fall into the host branch and read the
    // declaration as a host, and a prefix past the family's width must
    // never reach the mask calculation.
    for malformed in [
        "10.0.0.1/foo",
        "10.0.0.1/",
        "10.0.0.1/33",
        "10.0.0.1/64",
        "::1/129",
        "::1/200",
    ] {
        assert!(full_mask(malformed).is_err(), "{malformed}");
    }
}

/// A stub that keeps the rules of the chain, one per line, and answers
/// the listing the way the firewall does.
fn listing_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
    let file = dir.join("rules");
    std::fs::write(&file, rules.join("\n") + "\n").unwrap();
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -D) needle=\"$3 $4 $5 $6\"; awk -v n=\"$needle\" 'BEGIN{{done=0}} {{if (!done && index($0, n) > 0) {{done=1; next}} print}}' '{file}' > '{file}.tmp' && mv '{file}.tmp' '{file}'; exit 0;;\n  -C) exit 1;;\nesac\nexit 0\n",
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
    let spec = rule_spec(params.as_object().unwrap()).unwrap();
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

#[test]
fn a_rule_with_another_matcher_is_not_the_declared_match() {
    let dir = scratch("other-matcher");
    // The rule of the chain carries a matcher the declared match does not
    // (`--dport 22`): it is another rule, and it stays.
    let (binary, file) = listing_stub(&dir, &["-p tcp --dport 22 -j ACCEPT"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "jump":"DROP", "state":"absent"
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
    assert_eq!(left, "-p tcp --dport 22 -j ACCEPT\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_quoted_comment_is_one_argument() {
    let dir = scratch("quoted-comment");
    // The listing quotes a value with whitespace in it, the way
    // `iptables-save` prints it.
    let (binary, file) = listing_stub(
        &dir,
        &["-p tcp -d 10.0.0.1/32 -m comment --comment \"allow web\" -j DROP"],
    );
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.1", "comment":"allow web",
        "jump":"DROP", "state":"absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_malformed_destination_prefix_is_refused() {
    let dir = scratch("malformed");
    // The declaration names a prefix that is not a number: it is not the
    // host rule the chain holds, and the removal must refuse it instead
    // of reading it as the host and deleting the rule.
    let (binary, file) = listing_stub(&dir, &["-p tcp -d 10.0.0.1/32 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.1/foo", "jump":"DROP",
        "state":"absent"
    });
    let error = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap_err();
    assert!(error.contains("10.0.0.1/foo"), "{error}");
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -d 10.0.0.1/32 -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_out_of_range_destination_prefix_is_refused() {
    let dir = scratch("out-of-range");
    // `/33` is past IPv4's width: the mask calculation must never see
    // it, and the rule the chain holds is not the declaration.
    let (binary, file) = listing_stub(&dir, &["-p tcp -d 10.0.0.1/32 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.1/33", "jump":"DROP",
        "state":"absent"
    });
    let error = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap_err();
    assert!(error.contains("10.0.0.1/33"), "{error}");
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -d 10.0.0.1/32 -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_wildcard_destination_is_the_omitted_default() {
    let dir = scratch("wildcard");
    // The universal destination is the omitted one: `/0` has the same
    // effect as omitting the address option, and the listing leaves it
    // out, so the rule the removal declares is the rule in the chain.
    let (binary, file) = listing_stub(&dir, &["-p tcp -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"0.0.0.0/0", "jump":"DROP",
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
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_destination_family_is_validated_before_a_wildcard_is_omitted() {
    let dir = scratch("cross-family");
    // The IPv6 wildcard is not the omitted destination of the IPv4 rule
    // set: `iptables` and `ip6tables` hold their own rules, so the
    // opposite-family address must be refused before the wildcard branch
    // erases it and the removal takes the no-destination rule of the
    // chain.
    let (binary, file) = listing_stub(&dir, &["-p tcp -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"::/0", "jump":"DROP",
        "state":"absent"
    });
    let error = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap_err();
    assert!(error.contains("::/0"), "{error}");
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_ipv6_rule_refuses_an_ipv4_destination() {
    let dir = scratch("cross-family-v6");
    // The mirror: the IPv4 wildcard of an IPv6 rule is refused, and the
    // no-destination rule of the IPv6 chain stays.
    let (binary, file) = listing_stub(&dir, &["-p tcp -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"0.0.0.0/0", "jump":"DROP",
        "ip_version":"ipv6", "state":"absent"
    });
    let error = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap_err();
    assert!(error.contains("0.0.0.0/0"), "{error}");
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(left, "-p tcp -j DROP\n", "{left}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_wildcard_destination_does_not_take_a_rule_with_another_destination() {
    let dir = scratch("wildcard-other");
    // The wildcard is the omitted destination, not every destination: a
    // rule that names a network is another match and stays.
    let (binary, file) = listing_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"0.0.0.0/0", "jump":"DROP",
        "state":"absent"
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
fn a_masked_destination_is_compared_the_way_the_firewall_prints_it() {
    let dir = scratch("masked");
    // The firewall masks the host bits off the address it is given, so
    // the rule it prints is the network, and that is what the removal
    // compares.
    let (binary, file) = listing_stub(&dir, &["-p tcp -d 10.0.0.0/8 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "destination":"10.0.0.1/8", "jump":"DROP",
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
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_goto_rule_is_the_declared_match_of_absent_mode() {
    let dir = scratch("goto");
    // The listing hands the packet to another chain: `-g` ends the match
    // the way `-j` does, so the rule is the declared match and absent
    // mode takes it away instead of reporting it gone.
    let (binary, file) = listing_stub(&dir, &["-p tcp -g ALLOW"]);
    let params = json!({
        "chain":"INPUT", "protocol":"tcp", "jump":"DROP", "state":"absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn protocol_zero_is_the_omitted_default_of_absent_mode() {
    let dir = scratch("proto-zero");
    // `iptables` reads protocol `0` as `all` and prints no `-p` for it,
    // so the declared zero is the listed rule without a protocol: absent
    // mode takes it away instead of reporting it gone.
    let (binary, file) = listing_stub(&dir, &["-d 10.0.0.0/8 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "protocol":"0", "destination":"10.0.0.0/8", "jump":"DROP",
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
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_ipv6_zone_is_dropped_before_the_match_is_compared() {
    let dir = scratch("zone");
    // The firewall stores the address and the mask but not the zone, so
    // the declared zone is the listed rule without one: absent mode takes
    // it away instead of reporting it gone.
    let (binary, file) = listing_stub(&dir, &["-d fe80::1/128 -j DROP"]);
    let params = json!({
        "chain":"INPUT", "destination":"fe80::1%eth0", "jump":"DROP",
        "ip_version":"ipv6", "state":"absent"
    });
    let result = run_with(
        &binary,
        "INPUT",
        params.as_object().unwrap(),
        &context(false),
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

//! The match of a rule: the options the target carries are not part of
//! it.

#[test]
fn a_target_option_is_not_a_match_criterion() {
    // iptables prints the options of the target after it (`--reject-with`
    // and the like): they belong to the target, and a rule is not a
    // different match for carrying them. A REJECT rule would otherwise
    // not be the rule an absent step looks for, and would stay active
    // while the removal reports success.
    let declared =
        super::super::rules::match_pairs(&["-p", "tcp", "-j", "REJECT"].map(String::from));
    let listed = super::super::rules::match_pairs(
        &[
            "-p",
            "tcp",
            "-j",
            "REJECT",
            "--reject-with",
            "icmp-port-unreachable",
        ]
        .map(String::from),
    );
    assert_eq!(listed, declared);
}

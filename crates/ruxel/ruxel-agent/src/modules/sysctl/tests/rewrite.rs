//! The entry rewrite: canonical entry-key matching across the two
//! spellings of one parameter, and the value's own equals signs.

#[test]
fn whitespace_normalization() {
    assert_eq!(
        super::super::normalized("1024\t65535"),
        super::super::normalized("1024 65535")
    );
    assert_eq!(super::super::normalized(" 1 "), "1");
}

#[test]
fn rewrites_existing_key_or_appends_missing_key() {
    let current = "# keep\nnet.test = old\n";
    assert_eq!(
        super::super::rewrite_file(current, "net.test", "new"),
        ("# keep\nnet.test=new\n".into(), true)
    );
    assert_eq!(
        super::super::rewrite_file("# keep\n", "net.test", "new"),
        ("# keep\nnet.test=new\n".into(), true)
    );
    assert_eq!(
        super::super::rewrite_file("net.test = 1  2\n", "net.test", "1\t2"),
        ("net.test = 1  2\n".into(), false)
    );
}

#[test]
fn an_alias_spelling_is_the_same_entry() {
    // Slash-first and dotted spellings address one kernel parameter, so
    // the file's alias line is the same entry and is rewritten too:
    // updating only one spelling would let boot process the later alias
    // and restore the old value.
    assert_eq!(
        super::super::rewrite_file(
            "net.ipv4.ip_forward=0\nnet/ipv4/ip_forward=1\n",
            "net.ipv4.ip_forward",
            "0"
        ),
        (
            "net.ipv4.ip_forward=0\nnet.ipv4.ip_forward=0\n".into(),
            true
        )
    );
    // An alias line alone is rewritten to the managed spelling, with no
    // duplicate entry appended.
    assert_eq!(
        super::super::rewrite_file("net/ipv4/ip_forward=1\n", "net.ipv4.ip_forward", "0"),
        ("net.ipv4.ip_forward=0\n".into(), true)
    );
    // A slash-first name's literal dot keeps its identity apart from the
    // dotted spelling of another path (`enp3s0.200` is not
    // `enp3s0/200`), so no false entry matches.
    assert_eq!(
        super::super::rewrite_file(
            "net.ipv4.conf.enp3s0.200.forwarding=1\n",
            "net/ipv4/conf/enp3s0.200/forwarding",
            "0"
        ),
        (
            "net.ipv4.conf.enp3s0.200.forwarding=1\nnet/ipv4/conf/enp3s0.200/forwarding=0\n".into(),
            true
        )
    );
}

#[test]
fn an_equals_sign_in_the_value_is_not_a_delimiter() {
    // A `kernel.core_pattern` command carries `--option=value`: the
    // entry's delimiter is the FIRST equals sign, so comparing the
    // entry keeps the value's own equals signs whole. Splitting at
    // every equals sign truncated the value and made every apply see
    // the unchanged line as drift.
    let value = "|/usr/lib/systemd/systemd-coredump --option=value";
    assert_eq!(
        super::super::rewrite_file(
            "kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n",
            "kernel.core_pattern",
            value,
        ),
        (
            "kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n".into(),
            false
        )
    );
}

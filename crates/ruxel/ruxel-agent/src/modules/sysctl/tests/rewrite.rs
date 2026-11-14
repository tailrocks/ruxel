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
    let current = b"# keep\nnet.test = old\n";
    assert_eq!(
        super::super::rewrite_file(current, "net.test", "new"),
        (b"# keep\nnet.test=new\n".to_vec(), true)
    );
    assert_eq!(
        super::super::rewrite_file(b"# keep\n", "net.test", "new"),
        (b"# keep\nnet.test=new\n".to_vec(), true)
    );
    assert_eq!(
        super::super::rewrite_file(b"net.test = 1  2\n", "net.test", "1\t2"),
        (b"net.test = 1  2\n".to_vec(), false)
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
            b"net.ipv4.ip_forward=0\nnet/ipv4/ip_forward=1\n",
            "net.ipv4.ip_forward",
            "0"
        ),
        (
            b"net.ipv4.ip_forward=0\nnet.ipv4.ip_forward=0\n".to_vec(),
            true
        )
    );
    // An alias line alone is rewritten to the managed spelling, with no
    // duplicate entry appended.
    assert_eq!(
        super::super::rewrite_file(b"net/ipv4/ip_forward=1\n", "net.ipv4.ip_forward", "0"),
        (b"net.ipv4.ip_forward=0\n".to_vec(), true)
    );
    // A slash-first name's literal dot keeps its identity apart from the
    // dotted spelling of another path (`enp3s0.200` is not
    // `enp3s0/200`), so no false entry matches.
    assert_eq!(
        super::super::rewrite_file(
            b"net.ipv4.conf.enp3s0.200.forwarding=1\n",
            "net/ipv4/conf/enp3s0.200/forwarding",
            "0"
        ),
        (
            b"net.ipv4.conf.enp3s0.200.forwarding=1\nnet/ipv4/conf/enp3s0.200/forwarding=0\n"
                .to_vec(),
            true
        )
    );
}

#[test]
fn a_dash_prefixed_entry_is_the_same_entry() {
    // procps reads a leading '-' as the ignore-failure marker: the
    // assignment happens either way, so the line is the entry itself. A
    // rewrite that did not see it would leave the marked assignment in
    // the file and append a second assignment of the same key beside
    // it — the file then holds two values for one parameter, and the
    // boot value is whichever line comes last.
    assert_eq!(
        super::super::rewrite_file(b"-net.ipv4.ip_forward = 0\n", "net.ipv4.ip_forward", "1"),
        (b"-net.ipv4.ip_forward=1\n".to_vec(), true)
    );
    // The marker belongs to the line, so it survives the rewrite: the
    // host's "ignore errors" intent is not dropped.
    assert_eq!(
        super::super::rewrite_file(b"-net.ipv4.ip_forward=1\n", "net.ipv4.ip_forward", "1"),
        (b"-net.ipv4.ip_forward=1\n".to_vec(), false)
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
            b"kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n",
            "kernel.core_pattern",
            value,
        ),
        (
            b"kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n".to_vec(),
            false
        )
    );
}

#[test]
fn unrelated_bytes_survive_the_rewrite_verbatim() {
    // The rewrite carries the host's content as raw bytes: a comment
    // that is not valid text — a byte procps ignores all the same —
    // must come back byte-for-byte, and a line the rewrite does not
    // touch keeps its own bytes and line ending.
    let current = b"# note \xff\r\nnet.other=1\r\n";
    let (content, changed) = super::super::rewrite_file(current, "net.test", "1");
    assert!(changed);
    assert_eq!(
        content,
        b"# note \xff\r\nnet.other=1\r\nnet.test=1\n".to_vec(),
        "the unrelated bytes and line endings survive: {content:?}"
    );
    // The managed entry is matched across the line ending's bytes: a
    // CRLF entry line is the same entry and is rewritten to the
    // managed spelling.
    let (content, changed) = super::super::rewrite_file(b"net.test=0\r\n", "net.test", "1");
    assert!(changed);
    assert_eq!(content, b"net.test=1\n".to_vec(), "{content:?}");
    // A byte-identical file publishes nothing.
    let (content, changed) =
        super::super::rewrite_file(b"# note \xff\nnet.test=1\n", "net.test", "1");
    assert!(!changed);
    assert_eq!(content, b"# note \xff\nnet.test=1\n".to_vec());
}

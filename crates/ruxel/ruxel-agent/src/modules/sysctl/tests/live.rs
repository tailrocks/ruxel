//! The live comparison: edge whitespace is value content and
//! compares exactly, while the separator whitespace of vector values
//! compares token-wise — `/proc/sys` separates numeric lists with
//! tabs where workflows write spaces.

#[test]
fn tab_separated_vector_values_match_space_separated_ones() {
    // `net.ipv4.ip_local_port_range` reads back tab-separated from
    // `/proc/sys` while the workflow writes it space-separated: the
    // two spellings hold the same list, so no change is reported and
    // no `sysctl -w` runs on an idempotent apply.
    assert!(super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768\t60999",
        "32768 60999"
    ));
    assert!(super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768  60999",
        "32768\t60999"
    ));
    assert!(super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "4\t4\t1\t7",
        "4 4 1 7"
    ));
}

#[test]
fn vector_values_still_compare_exactly_where_they_differ() {
    // Token-wise is not value-blind: another list, another arity, or
    // a non-numeric token is a different value and is reconciled.
    assert!(!super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768\t60999",
        "32768 61000"
    ));
    assert!(!super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768\t60999",
        "32768"
    ));
    assert!(!super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768\t60999",
        "32768 60999 1"
    ));
    assert!(!super::super::live_matches(
        "kernel.core_pattern",
        "abc\tdef",
        "abc def"
    ));
}

#[test]
fn scalar_strings_keep_every_blank() {
    // The blanks inside a scalar string are part of the kernel value:
    // `kernel.hostname` holds "abc  def" as written, so collapsing
    // the separators would skip a write the host still needs.
    assert!(!super::super::live_matches(
        "kernel.hostname",
        "abc  def",
        "abc def"
    ));
    assert!(!super::super::live_matches(
        "kernel.core_pattern",
        "abc\tdef",
        "abc def"
    ));
    assert!(super::super::live_matches(
        "kernel.hostname",
        "abc  def",
        "abc  def"
    ));
}

#[test]
fn edge_whitespace_compares_exactly() {
    // procps strips ASCII blanks around values (`rstrip`), so "1 "
    // holds the same value as "1" — but non-ASCII edge whitespace is
    // content the byte-level trim preserves: "nb\u{a0}" is another value
    // than "nb", reconciled rather than skipped.
    assert!(super::super::live_matches("kernel.hostname", "1 ", "1"));
    assert!(super::super::live_matches("kernel.hostname", " 1 ", " 1 "));
    assert!(super::super::live_matches("kernel.hostname", "1", "1"));
    assert!(!super::super::live_matches(
        "kernel.hostname",
        "nb\u{a0}",
        "nb"
    ));
    assert!(super::super::live_matches(
        "kernel.hostname",
        "nb\u{a0}",
        "nb\u{a0}"
    ));
}

#[test]
fn the_proc_line_ending_is_not_value_content() {
    // The kernel terminates the `/proc/sys` value with `\n`: the live
    // path strips that ending before the syntax-blank trim, so a value
    // the workflow wrote reads back equal and no `sysctl -w` runs on
    // an idempotent apply — while another value under the same ending
    // still reconciles.
    assert!(super::super::live_matches("kernel.hostname", "1\n", "1"));
    assert!(super::super::live_matches("kernel.hostname", "1\r\n", "1"));
    assert!(super::super::live_matches(
        "net.ipv4.ip_local_port_range",
        "32768\t60999\n",
        "32768 60999"
    ));
    assert!(!super::super::live_matches("kernel.hostname", "2\n", "1"));
}

#[test]
fn the_live_read_strips_the_proc_line_ending() {
    // The exact mapping `read_sysctl` applies to the `/proc/sys`
    // bytes, proved without `/proc/sys`: a "1\n" payload reads back as
    // "1", so the value the first apply wrote compares equal on the
    // second and the reapply is a no-op.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-live-read-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    for (payload, want) in [("1\n", "1"), ("1\r\n", "1"), ("\u{B}1\u{B}\n", "1")] {
        let path = dir.join("live");
        std::fs::write(&path, payload).expect("operation succeeds");
        let text = path.to_str().expect("path is text").to_owned();
        let reread = super::super::read_live(&text).expect("the live file reads");
        assert_eq!(reread, want, "the read strips the ending: {payload:?}");
        assert!(
            super::super::live_matches("kernel.hostname", &reread, "1"),
            "the reread value is a no-op against the desired one"
        );
    }
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn string_values_compare_whitespace_exactly() {
    // Thread 318: a string-valued parameter whose value holds several
    // numeric-looking words is still text — `kernel.core_pattern` keeps
    // every blank of `12  34`, so a live `12 34` is drift, not a
    // separator spelling, and the apply must reconcile it.
    assert!(!super::super::live_matches(
        "kernel.core_pattern",
        "12  34",
        "12 34"
    ));
    assert!(super::super::live_matches(
        "kernel.core_pattern",
        "12  34",
        "12  34"
    ));
}

#[test]
fn the_file_trim_keeps_the_line_ending_outside_its_set() {
    // Pin: the fix touches the live path only — the file-side trims
    // keep their four syntax blanks, and `\n` is not one of them, so
    // the byte-level non-UTF8 preservation is unchanged.
    assert_eq!(super::super::trim(b"1\n"), b"1\n");
    assert_eq!(super::super::trimmed_value("1\n"), "1\n");
}

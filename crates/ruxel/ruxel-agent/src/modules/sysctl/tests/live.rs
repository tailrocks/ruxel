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
    assert!(super::super::live_matches("32768\t60999", "32768 60999"));
    assert!(super::super::live_matches("32768  60999", "32768\t60999"));
    assert!(super::super::live_matches("4\t4\t1\t7", "4 4 1 7"));
}

#[test]
fn vector_values_still_compare_exactly_where_they_differ() {
    // Token-wise is not value-blind: another list, another arity, or
    // a non-numeric token is a different value and is reconciled.
    assert!(!super::super::live_matches("32768\t60999", "32768 61000"));
    assert!(!super::super::live_matches("32768\t60999", "32768"));
    assert!(!super::super::live_matches("32768\t60999", "32768 60999 1"));
    assert!(!super::super::live_matches("abc\tdef", "abc def"));
}

#[test]
fn scalar_strings_keep_every_blank() {
    // The blanks inside a scalar string are part of the kernel value:
    // `kernel.hostname` holds "abc  def" as written, so collapsing
    // the separators would skip a write the host still needs.
    assert!(!super::super::live_matches("abc  def", "abc def"));
    assert!(!super::super::live_matches("abc\tdef", "abc def"));
    assert!(super::super::live_matches("abc  def", "abc  def"));
}

#[test]
fn edge_whitespace_compares_exactly() {
    // procps strips ASCII blanks around values (`rstrip`), so "1 "
    // holds the same value as "1" — but non-ASCII edge whitespace is
    // content the byte-level trim preserves: "nb\u{a0}" is another value
    // than "nb", reconciled rather than skipped.
    assert!(super::super::live_matches("1 ", "1"));
    assert!(super::super::live_matches(" 1 ", " 1 "));
    assert!(super::super::live_matches("1", "1"));
    assert!(!super::super::live_matches("nb\u{a0}", "nb"));
    assert!(super::super::live_matches("nb\u{a0}", "nb\u{a0}"));
}

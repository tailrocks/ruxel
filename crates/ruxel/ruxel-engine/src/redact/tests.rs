use super::*;

#[test]
fn redacts_plain_base64_lines_and_skips_short_values() {
    let r = Redactor::new(["secret-key", "abcde", "first-line\nsecond-line"]);
    for text in [
        "secret-key",
        &STANDARD.encode("secret-key"),
        "c2VjcmV0LWtleQ",
        "second-line",
    ] {
        assert_eq!(r.redact(text), "[REDACTED]");
    }
    assert_eq!(r.redact("abcde"), "abcde");
}

#[test]
fn no_match_is_borrowed_and_extend_adds_patterns() {
    let mut r = Redactor::empty();
    assert!(matches!(r.redact("clean"), Cow::Borrowed(_)));
    r.extend(["secret-value"]);
    assert_eq!(r.redact("secret-value"), "[REDACTED]");
    assert!(!format!("{r:?}").contains("secret-value"));
}

#[test]
fn overlaps_are_replaced_as_one_match() {
    let r = Redactor::new(["secret-value", "secret"]);
    assert_eq!(r.redact("secret-value"), "[REDACTED]");
}

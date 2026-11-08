use super::*;

#[test]
fn scan_intersection() {
    assert!(!deny_hits(&policy(), &["192.0.2.0/24"]).is_empty());
}
#[test]
fn scan_alternate_numeric_addresses() {
    let deny = Policy::new(&[], &["127.0.0.0/8".into()]).unwrap();
    for value in [
        "2130706433",
        "017700000001",
        "0x7f000001",
        "127.1",
        "http://0x7f000001:80/",
        "http://0x7f000001/path",
        "https://user:password@0x7f000001/path",
        "2130706433/a/b",
        "127.1/path",
    ] {
        assert!(!deny_hits(&deny, &[value]).is_empty(), "{value}");
    }
    assert!(deny_hits(&deny, &["0x100000000", "1.2.3.999"]).is_empty());
}

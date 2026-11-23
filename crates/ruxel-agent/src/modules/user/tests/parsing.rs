//! The passwd and group parsers over canned content.

use super::super::*;
use super::strings;

#[test]
fn parses_passwd_and_group_content() {
    let entry = passwd_entry_from("alice:x:1001:1002:Alice:/home/alice:/bin/sh\n", "alice")
        .unwrap()
        .unwrap();
    assert_eq!(
        (entry.uid, entry.gid, entry.home.as_str()),
        (1001, 1002, "/home/alice")
    );
    assert_eq!(
        supplementary_groups_from("wheel:x:10:alice,bob\ndocker:x:20:bob\n", "alice"),
        strings(&["wheel"])
    );
}

#[test]
fn parses_group_entry_from_canned_content() {
    assert_eq!(
        super::group_entry_from("wheel:x:10:a,b\n", "wheel").unwrap(),
        Some(("wheel".into(), 10))
    );
    assert_eq!(
        super::group_entry_from("wheel:x:10:a,b\n", "missing").unwrap(),
        None
    );
    assert!(super::group_entry_from("broken:x:nope:\n", "broken").is_err());
}

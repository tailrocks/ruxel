#[test]
fn the_system_range_comes_from_the_host_policy() {
    // Thread 336: the boundary is the host's configured range, not the
    // fixed 1000. The last uncommented line per key wins — the
    // `USERGROUPS_ENAB` pattern — with trailing comments stripped, and
    // a statically assigned system uid below the floor is still the
    // host's to keep: the ceiling, not the floor, is the boundary.
    let _policy = super::pin_test_policy(
        "# a comment\nSYS_UID_MIN 100\nSYS_UID_MAX 999 # trailing\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n",
    );
    let ranges = super::system_id_ranges().expect("the pinned policy parses");
    assert_eq!((ranges.uid_min, ranges.uid_max), (100, 999));
    assert_eq!((ranges.gid_min, ranges.gid_max), (100, 999));
    assert!(super::system_uid_kept(1, &ranges));
    assert!(super::system_uid_kept(999, &ranges));
    assert!(!super::system_uid_kept(1000, &ranges));
    assert!(super::system_gid_kept(999, &ranges));
    assert!(!super::system_gid_kept(1000, &ranges));
    let entry = |uid: u64| super::super::PasswdEntry {
        uid,
        ..super::super::PasswdEntry::default()
    };
    assert!(
        super::removal_allowed("daemon", None, Some(&entry(1)), false).is_err(),
        "a statically assigned system uid below the floor is the host's to keep"
    );
    assert!(
        super::removal_allowed("svc", None, Some(&entry(999)), false).is_err(),
        "the ceiling itself is the host's to keep"
    );
    assert!(
        super::removal_allowed("app", None, Some(&entry(1000)), false).is_ok(),
        "one above the ceiling is a login uid"
    );
    assert!(
        super::group_removal_allowed("svc", None, 999, None).is_err(),
        "the gid ceiling is the host's to keep too"
    );
    assert!(
        super::group_removal_allowed("app", None, 1000, None).is_ok(),
        "one above the gid ceiling is a login group"
    );
}

#[test]
fn the_system_range_resolves_like_usergroups_enab() {
    // The last uncommented line per key wins; a commented-out line and
    // a missing, unparsable or inverted key parse as no range, which
    // fails the removal closed downstream.
    let ranges = super::system_id_ranges_from(
        "SYS_UID_MIN 100\n#SYS_UID_MIN 500\nSYS_UID_MIN 200\nSYS_UID_MAX 1999\nSYS_GID_MIN 200\nSYS_GID_MAX 1999\n",
    )
    .expect("four parsable numbers parse");
    assert_eq!((ranges.uid_min, ranges.uid_max), (200, 1999));
    assert!(super::system_id_ranges_from("SYS_UID_MIN 100\n").is_none());
    assert!(
        super::system_id_ranges_from(
            "SYS_UID_MIN 100\nSYS_UID_MAX many\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n"
        )
        .is_none(),
        "an unparsable key is no range"
    );
    assert!(
        super::system_id_ranges_from(
            "SYS_UID_MIN 1000\nSYS_UID_MAX 999\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n"
        )
        .is_none(),
        "a floor above its ceiling is no range"
    );
}

#[test]
fn other_primary_holders_follow_a_renumbered_group() {
    // Alice's private group lives at gid 1000 and Bob holds it as
    // his primary group; the plan renumbers the group to 2000. The
    // apply moves the group first (`groupmod -g`), which gives every
    // holder the new number, so under the plan Bob holds 2000 and
    // nothing holds 1000 — while the host's passwd entry still shows
    // the old number.
    let passwd = "alice:x:1001:1000::/home/alice:/bin/sh\n\
                      bob:x:1002:1000::/home/bob:/bin/sh\n";
    assert_eq!(
        super::other_primary_group_holders_from(passwd, 1000, "alice", "alice"),
        vec!["bob".to_string()],
        "before the plan Bob holds the live number"
    );
    super::super::plan::note_renumbered_group("alice", 1000, 2000);
    assert!(
        super::other_primary_group_holders_from(passwd, 1000, "alice", "alice").is_empty(),
        "the renumbered number is held by no one under the plan"
    );
    assert_eq!(
        super::other_primary_group_holders_from(passwd, 2000, "alice", "alice"),
        vec!["bob".to_string()],
        "Bob holds the new number under the plan"
    );
}

#[test]
fn private_group_emptiness_follows_planned_supplementary_membership() {
    // Alice's group is empty on the host: no live member lists it
    // and no other account holds its number as its primary group.
    let groups = "alice:x:1000:\n";
    let passwd = "alice:x:1001:1000::/home/alice:/bin/sh\n\
                      bob:x:1002:1003::/home/bob:/bin/sh\n";
    assert!(
        super::private_group_emptied_from(groups, passwd, "alice", Some(1000)),
        "the group no one holds is empty"
    );
    // An earlier task of the plan adds Bob to the group: check mode
    // runs no `usermod`, so the host's group file still shows no
    // member — but the apply adds Bob first and `userdel` preserves
    // the group, so the scan must not report it empty.
    super::super::plan::note_supplementary("bob", vec!["alice".to_string()]);
    assert!(
        !super::private_group_emptied_from(groups, passwd, "alice", Some(1000)),
        "the member the plan added keeps the group"
    );
    // The plan moves Bob off the group again: the live file is
    // unchanged either way, and the scan follows the plan's record.
    super::super::plan::note_supplementary("bob", Vec::new());
    assert!(
        super::private_group_emptied_from(groups, passwd, "alice", Some(1000)),
        "the member the plan moved away frees the group"
    );
}

#[test]
fn private_group_emptiness_skips_the_members_the_plan_removed() {
    // Bob is the group's live member, and the plan takes his account
    // away: check mode runs no `userdel`, so the host's group file
    // still lists him — but the apply removes him first and the
    // empty group goes with Alice.
    let groups = "alice:x:1000:bob\n";
    let passwd = "alice:x:1001:1000::/home/alice:/bin/sh\n\
                      bob:x:1002:1003::/home/bob:/bin/sh\n";
    assert!(
        !super::private_group_emptied_from(groups, passwd, "alice", Some(1000)),
        "the live member keeps the group"
    );
    super::super::entries::note_removed_account("bob");
    assert!(
        super::private_group_emptied_from(groups, passwd, "alice", Some(1000)),
        "the member the plan removed holds the group no longer"
    );
}

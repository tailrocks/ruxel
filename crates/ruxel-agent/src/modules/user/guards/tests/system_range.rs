#[test]
fn the_system_range_comes_from_the_host_policy() {
    // Thread 336: the boundary is the host's configured range, not the
    // fixed 1000. The last uncommented line per key wins — the
    // `USERGROUPS_ENAB` pattern — with trailing comments stripped, and
    // a statically assigned system uid below the floor is still the
    // host's to keep: the ceiling, not the floor, is the boundary.
    let _policy = super::super::pin_test_policy(
        "# a comment\nSYS_UID_MIN 100\nSYS_UID_MAX 999 # trailing\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n",
    );
    let ranges = super::super::system_id_ranges().expect("the pinned policy parses");
    assert_eq!((ranges.uid_min, ranges.uid_max), (100, 999));
    assert_eq!((ranges.gid_min, ranges.gid_max), (100, 999));
    assert!(super::super::system_uid_kept(1, &ranges));
    assert!(super::super::system_uid_kept(999, &ranges));
    assert!(!super::super::system_uid_kept(1000, &ranges));
    assert!(super::super::system_gid_kept(999, &ranges));
    assert!(!super::super::system_gid_kept(1000, &ranges));
    let entry = |uid: u64| super::super::super::PasswdEntry {
        uid,
        ..super::super::super::PasswdEntry::default()
    };
    assert!(
        super::super::removal_allowed("daemon", None, Some(&entry(1)), false).is_err(),
        "a statically assigned system uid below the floor is the host's to keep"
    );
    assert!(
        super::super::removal_allowed("svc", None, Some(&entry(999)), false).is_err(),
        "the ceiling itself is the host's to keep"
    );
    assert!(
        super::super::removal_allowed("app", None, Some(&entry(1000)), false).is_ok(),
        "one above the ceiling is a login uid"
    );
    assert!(
        super::super::group_removal_allowed("svc", None, 999, None).is_err(),
        "the gid ceiling is the host's to keep too"
    );
    assert!(
        super::super::group_removal_allowed("app", None, 1000, None).is_ok(),
        "one above the gid ceiling is a login group"
    );
}

#[test]
fn the_system_range_resolves_like_usergroups_enab() {
    // The last uncommented line per key wins; a commented-out line is
    // not a line — while a present but unparsable key, or a floor
    // above its ceiling, parses as no range, which fails the removal
    // closed downstream. A key the content omits is not unparsable:
    // it falls back to the shadow-utils default (see below).
    let ranges = super::super::system_id_ranges_from(
        "SYS_UID_MIN 100\n#SYS_UID_MIN 500\nSYS_UID_MIN 200\nSYS_UID_MAX 1999\nSYS_GID_MIN 200\nSYS_GID_MAX 1999\n",
    )
    .expect("four parsable numbers parse");
    assert_eq!((ranges.uid_min, ranges.uid_max), (200, 1999));
    assert!(
        super::super::system_id_ranges_from(
            "SYS_UID_MIN 100\nSYS_UID_MAX many\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n"
        )
        .is_none(),
        "an unparsable key is no range"
    );
    assert!(
        super::super::system_id_ranges_from(
            "SYS_UID_MIN 1000\nSYS_UID_MAX 999\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n"
        )
        .is_none(),
        "a floor above its ceiling is no range"
    );
}

#[test]
fn omitted_range_keys_fall_back_to_shadow_utils_defaults() {
    // Thread 359: a login.defs that omits the range keys is not a
    // missing policy — shadow-utils draws system accounts from
    // 100-999 by default, so the guard keeps that range instead of
    // failing closed on the missing keys.
    let ranges = super::super::system_id_ranges_from("SYS_UID_MIN 100\n")
        .expect("an omitted key falls back to its default");
    assert_eq!(
        (ranges.uid_min, ranges.uid_max),
        (100, 999),
        "the present floor stays, the omitted ceiling defaults"
    );
    assert_eq!(
        (ranges.gid_min, ranges.gid_max),
        (100, 999),
        "the omitted gid keys default"
    );
    let ranges = super::super::system_id_ranges_from("# no range keys at all\n")
        .expect("no keys still parse");
    assert_eq!((ranges.uid_min, ranges.uid_max), (100, 999));
    assert_eq!((ranges.gid_min, ranges.gid_max), (100, 999));
    // The boundary still guards removals under the defaulted policy.
    let _policy = super::super::pin_test_policy("# no range keys at all\n");
    let entry = |uid: u64| super::super::super::PasswdEntry {
        uid,
        ..super::super::super::PasswdEntry::default()
    };
    assert!(
        super::super::removal_allowed("svc", None, Some(&entry(999)), false).is_err(),
        "the default ceiling itself is the host's to keep"
    );
    assert!(
        super::super::removal_allowed("app", None, Some(&entry(1000)), false).is_ok(),
        "one above the default ceiling is a login uid"
    );
    assert!(
        super::super::group_removal_allowed("svc", None, 999, None).is_err(),
        "the default gid ceiling is the host's to keep too"
    );
    assert!(
        super::super::group_removal_allowed("app", None, 1000, None).is_ok(),
        "one above the default gid ceiling is a login group"
    );
}

#[test]
fn an_omitted_system_ceiling_comes_from_the_regular_floor() {
    // The review's P1 on this PR: shadow-utils draws an omitted
    // SYS_UID_MAX from UID_MIN - 1 (and SYS_GID_MAX from GID_MIN -
    // 1), not from the 999 that only holds where the regular floor
    // is the 1000 default. UID_MIN 2000 allocates system accounts
    // up to 1999, which the guard keeps.
    let ranges = super::super::system_id_ranges_from("UID_MIN 2000\nGID_MIN 2000\n")
        .expect("the omitted ceilings derive from the regular floors");
    assert_eq!((ranges.uid_min, ranges.uid_max), (100, 1999));
    assert_eq!((ranges.gid_min, ranges.gid_max), (100, 1999));
    assert!(super::super::system_uid_kept(1500, &ranges));
    assert!(!super::super::system_uid_kept(2000, &ranges));
    let _policy = super::super::pin_test_policy("UID_MIN 2000\nGID_MIN 2000\n");
    let entry = |uid: u64| super::super::super::PasswdEntry {
        uid,
        ..super::super::super::PasswdEntry::default()
    };
    assert!(
        super::super::removal_allowed("svc", None, Some(&entry(1500)), false).is_err(),
        "a uid inside the derived system range is the host's to keep"
    );
}

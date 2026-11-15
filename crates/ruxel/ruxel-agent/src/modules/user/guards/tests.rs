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

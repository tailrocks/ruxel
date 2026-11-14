//! The guards that read the host's own passwd database: the account
//! that holds a group as its primary group, and the accounts the plan
//! has already removed.

#[test]
fn a_primary_group_holder_the_plan_removed_is_no_holder() {
    // Check mode runs no `userdel`, so the live passwd file still
    // lists an account the plan has removed; the group's removal must
    // skip that account and keep looking for a remaining holder —
    // apply deletes the account first and its `groupdel` then
    // succeeds, so the dry run must not refuse the removal.
    let passwd = "ruxel-removed-holder:x:1001:4242::/home/ruxel-removed-holder:/bin/sh\n\
                  ruxel-remaining-holder:x:1002:4242::/home/ruxel-remaining-holder:/bin/sh\n";
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 4242, "staff"),
        Some("ruxel-removed-holder".to_string()),
        "the first holder is the account the passwd file lists"
    );
    super::super::entries::note_removed_account("ruxel-removed-holder");
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 4242, "staff"),
        Some("ruxel-remaining-holder".to_string()),
        "the removed account is skipped and the scan continues"
    );
    super::super::entries::note_removed_account("ruxel-remaining-holder");
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 4242, "staff"),
        None,
        "no holder remains once every holder is removed"
    );
}

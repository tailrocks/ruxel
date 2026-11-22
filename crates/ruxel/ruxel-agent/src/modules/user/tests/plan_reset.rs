//! A new plan clears the previous plan's user/group record, while a
//! patch continues the plan it extends: in check mode a first plan
//! requesting UID 1600 records that effective UID without changing
//! `/etc/passwd`, and a second plan requesting the same UID must
//! compare against the host's own entry again — not against the stale
//! planned value — and report the drift the host still holds.

use super::super::plan;
use super::super::*;

const ACCOUNT: &str = "ruxel-plan-reset-user";
const GROUP: &str = "ruxel-plan-reset-group";

/// The first plan's tasks declare the identities the plan manages.
fn declare_first_plan() {
    plan::note(GROUP, Some(1600));
    plan::note_user(ACCOUNT, Some(1600));
    plan::note_home(ACCOUNT, "/tmp/ruxel-plan-reset-home");
    plan::note_removed_account("ruxel-plan-reset-gone");
    plan::note_removed_group("ruxel-plan-reset-gone-group");
    plan::note_primary(
        ACCOUNT,
        plan::PlannedPrimary {
            name: GROUP.to_string(),
            gid: Some(1600),
        },
    );
    plan::note_supplementary(ACCOUNT, vec![GROUP.to_string()]);
    plan::note_renumbered_group(GROUP, 1600, 1601);
    plan::note_shell(ACCOUNT, Some("/bin/sh"));
    plan::note_comment(ACCOUNT, Some("reset"));
    plan::note_changed_user(ACCOUNT);
    plan::note_changed_group(GROUP);
    plan::note_system(ACCOUNT);
}

#[test]
fn two_plans_in_one_session_do_not_share_user_and_group_state() {
    // The first plan's declarations are all readable while the plan
    // runs: a patch frame of the same plan keeps them, since `serve`
    // resets only on `Msg::Plan` and never on `Msg::PlanPatch`.
    declare_first_plan();
    assert!(plan::group_requested(GROUP).is_some());
    assert!(plan::user_requested(ACCOUNT).is_some());
    assert!(plan::home(ACCOUNT).is_some());
    assert!(plan::account_removed("ruxel-plan-reset-gone"));
    assert!(plan::group_removed("ruxel-plan-reset-gone-group"));
    assert!(plan::primary(ACCOUNT).is_some());
    assert!(plan::supplementary(ACCOUNT).is_some());
    assert_eq!(plan::renumbered_gid(1600), Some(1601));
    assert!(plan::shell(ACCOUNT).is_some());
    assert!(plan::comment(ACCOUNT).is_some());
    assert!(plan::user_number_changed(ACCOUNT));
    assert!(plan::group_number_changed(GROUP));
    assert!(plan::is_system(ACCOUNT));
    // A second plan clears every map: the UID the first plan
    // recorded must not decide the second plan's comparison.
    begin_plan();
    assert!(plan::group_requested(GROUP).is_none());
    assert!(plan::user_requested(ACCOUNT).is_none());
    assert!(plan::home(ACCOUNT).is_none());
    assert!(!plan::account_removed("ruxel-plan-reset-gone"));
    assert!(!plan::group_removed("ruxel-plan-reset-gone-group"));
    assert!(plan::primary(ACCOUNT).is_none());
    assert!(plan::supplementary(ACCOUNT).is_none());
    assert_eq!(plan::renumbered_gid(1600), None);
    assert!(plan::shell(ACCOUNT).is_none());
    assert!(plan::comment(ACCOUNT).is_none());
    assert!(!plan::user_number_changed(ACCOUNT));
    assert!(!plan::group_number_changed(GROUP));
    assert!(!plan::is_system(ACCOUNT));
}

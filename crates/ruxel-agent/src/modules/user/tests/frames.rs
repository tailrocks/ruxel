//! The plan state across one session's per-task frames: the engine
//! frames every task as its own Plan, so the declarations and the
//! renumber markers of the first frame's tasks persist into the later
//! frames — the session's reset runs once, at its boundary, never
//! between frames.

use super::super::*;

const GROUP: &str = "ruxel-frame-group";
const ACCOUNT: &str = "ruxel-frame-user";

#[test]
fn planned_groups_stay_visible_into_later_frames() {
    // The session's boundary reset: exactly what serve() runs at Hello.
    begin_plan();
    // The first frame declares the group; the second frame — with no
    // reset between the frames — still resolves the account against it.
    plan::note(GROUP, Some(1700));
    assert!(plan::group_requested(GROUP).is_some());
}

#[test]
fn renumber_markers_survive_across_per_task_frames() {
    // The session's boundary reset: exactly what serve() runs at Hello.
    begin_plan();
    // The first frame renumbers the group — what group() records in
    // advance in check mode — and marks the changed numbers.
    plan::note_renumbered_group(GROUP, 1600, 1601);
    plan::note_changed_group(GROUP);
    plan::note_changed_user(ACCOUNT);
    // The second frame runs with no reset between the frames: the
    // markers are still there, so a file task naming the group or the
    // account is not served from a cache that weighs the old numbers
    // alone.
    assert!(group_number_changed(GROUP));
    assert!(user_number_changed(ACCOUNT));
    assert!(crate::modules::ownership_cache_stale(
        "file",
        &json!({"path": "/x", "group": GROUP}),
    ));
    assert!(crate::modules::ownership_cache_stale(
        "file",
        &json!({"path": "/x", "owner": ACCOUNT}),
    ));
}

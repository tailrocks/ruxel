//! A complete plan with no agent tasks: the workflow dropped its last
//! agent task, so the session encounters nothing — and the unfiltered
//! `Done` still marks the plan complete. The old records must expire:
//! a removed sysctl declaration left trusted would authorize its
//! dropped task's still-present later file. Only the unfiltered `Done`
//! completes a plan — the EOF and error arms of the session loop flush
//! without it — so a session that ends without one prunes nothing.

use super::*;

#[test]
fn a_complete_empty_plan_expires_its_workflow_records() {
    // Thread 339: the workflow dropped its last agent task, so the
    // replacement plan encounters no keys — and the controller still
    // sends an unfiltered `Done`, which completes the plan. The old
    // records are the dropped tasks' history and must expire, while
    // another workflow's records are no verdict of this plan's.
    let dir = Scratch::new("complete-empty-plan");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    ledger.begin_plan("workflow");
    record_file(&mut ledger, "dropped", &file);
    ledger.flush();
    let mut other = Ledger::load(&dir.0);
    other.begin_plan("other-workflow");
    record_file(&mut other, "elsewhere", &file);
    other.flush();

    let mut ledger = Ledger::load(&dir.0);
    ledger.begin_plan("workflow");
    ledger.complete_plan();
    ledger.flush();

    let mut probe = Ledger::load(&dir.0);
    probe.begin_plan("workflow");
    assert!(
        probe.cached_ok("dropped").is_none(),
        "the record of a task the completed workflow dropped must expire even where the plan encountered nothing"
    );
    probe.begin_plan("other-workflow");
    assert!(
        probe.cached_ok("elsewhere").is_some(),
        "another workflow's records survive a plan that is not theirs"
    );
}

#[test]
fn a_session_that_ends_without_done_prunes_nothing() {
    // The EOF and frame-error arms of the session loop flush the ledger
    // without completing the plan — only the unfiltered `Done` marks it
    // complete — so a session that ends that way is an executed prefix,
    // not a verdict, and expires nothing even where it encountered no
    // task of its own.
    let dir = Scratch::new("incomplete-session");
    let file = dir.file("target", b"stable");
    let mut ledger = Ledger::load(&dir.0);
    ledger.begin_plan("workflow");
    record_file(&mut ledger, "kept", &file);
    ledger.flush();

    let mut ledger = Ledger::load(&dir.0);
    ledger.begin_plan("workflow");
    ledger.flush();

    let mut probe = Ledger::load(&dir.0);
    probe.begin_plan("workflow");
    assert!(
        probe.cached_ok("kept").is_some(),
        "a session that never completed its plan must prune nothing"
    );
}

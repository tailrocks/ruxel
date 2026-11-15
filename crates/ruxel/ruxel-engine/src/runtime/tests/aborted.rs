//! The shutdown decision: a session whose end is no verdict on the
//! records of the tasks it never sent must expire nothing. Tag
//! selection omits tasks the workflow keeps (`filtered`), a
//! controller-side failure stops the session before its tasks are all
//! sent (`aborted`), and a failed run has no recap to consult at all.

use super::*;

fn recap(filtered: bool, aborted: bool) -> BTreeMap<String, Recap> {
    BTreeMap::from([(
        "h".to_string(),
        Recap {
            filtered,
            aborted,
            ..Recap::default()
        },
    )])
}

#[test]
fn a_plain_complete_session_stays_authoritative() {
    assert!(!session_incomplete(&Ok(recap(false, false)), "h"));
    assert!(!session_incomplete(&Ok(BTreeMap::new()), "h"));
}

#[test]
fn a_filtered_session_expires_nothing() {
    assert!(session_incomplete(&Ok(recap(true, false)), "h"));
}

#[test]
fn an_aborted_session_expires_nothing() {
    // The controller stopped before the session's tasks were all sent
    // — the later plays never ran — so the shutdown must not prune the
    // records of the tasks the agent never received.
    assert!(session_incomplete(&Ok(recap(false, true)), "h"));
}

#[test]
fn a_failed_run_expires_nothing() {
    // No recap survived to consult: the sessions that did open are an
    // executed prefix, not a complete plan, and expire nothing.
    let failed: Result<BTreeMap<String, Recap>> = Err(anyhow::anyhow!("synthetic failure"));
    assert!(session_incomplete(&failed, "h"));
}

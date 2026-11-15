//! Controller-aborted sessions stay incomplete: when evaluation or
//! rendering fails after some tasks ran, the later tasks are never
//! sent and the agent never calls `abort_plan` — so the shutdown must
//! not prune the records of the tasks the session never reached. The
//! session is marked aborted, and the mark rides the same no-prune
//! semantics as the tag-filtered mechanism.

use super::*;

fn two_tasks(second: &Value) -> Value {
    json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"first","operation":{"kind":"command","argv":["true"]}},
        second
    ]}]})
}

#[tokio::test]
async fn a_mid_play_evaluation_failure_marks_the_session_aborted() -> Result<()> {
    // The first task runs, the second fails to render: the agent never
    // receives it and never calls `abort_plan`, so the partial recap
    // must carry the aborted mark — without it the shutdown sends an
    // authoritative Done and prunes the never-sent task's records.
    let config = two_tasks(
        &json!({"name":"bad","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","missing"]}}}),
    );
    let mut backend = Fake::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await
    .err()
    .context("run should fail")?;
    let partial = error
        .downcast_ref::<RunFailure>()
        .context("missing partial recap")?;
    assert_eq!(partial.recaps["h"].ok, 1);
    assert_eq!(backend.calls.len(), 1);
    assert!(
        partial.recaps["h"].aborted,
        "a session stopped before its tasks were all sent is incomplete"
    );
    assert!(
        !partial.recaps["h"].filtered,
        "no tag selection ran: the mark is the abort, not the filter"
    );
    Ok(())
}

#[tokio::test]
async fn a_complete_run_marks_nothing_aborted() -> Result<()> {
    // Every task sent and answered: the session ran to its end, so its
    // shutdown stays an authoritative verdict on the records it saw.
    let config =
        two_tasks(&json!({"name":"second","operation":{"kind":"command","argv":["true"]}}));
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recaps["h"].ok, 2);
    assert!(
        !recaps["h"].aborted,
        "a complete session must stay authoritative"
    );
    Ok(())
}

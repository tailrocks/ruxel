//! An incomplete origin marks its delegation destinations incomplete
//! too: a delegated host with a clean direct recap of its own must
//! still shut down filtered, or its unfiltered Done prunes delegated
//! records the origin never resent.

use super::*;

#[test]
fn literal_delegation_targets_cover_nested_blocks() {
    // A skipped task never evaluates its destination, so the literal
    // targets are read play-wide — including nested blocks — whatever
    // skips the origin before anything is sent.
    let play = json!({"targets": ["a"], "tasks": [
        {"name": "direct", "delegateTo": "b", "delegateAllow": ["b"], "operation": {"kind": "command", "argv": ["true"]}},
        {"name": "block", "block": [
            {"name": "nested", "delegateTo": "c", "delegateAllow": ["c"], "operation": {"kind": "command", "argv": ["true"]}}
        ]}
    ]});
    assert_eq!(
        super::super::delegation::literal_targets(&play),
        BTreeSet::from(["b".to_string(), "c".to_string()]),
        "nested literal destinations are propagation targets too"
    );
}

#[tokio::test]
async fn a_filtered_origin_marks_its_delegation_destination() -> Result<()> {
    // Thread 333: the previous run delegated task X to B; this run
    // skips X on A by tag while B — also a play target — runs a clean
    // direct task. B's recap must carry the incompleteness, or B's
    // shutdown reads only its unrelated clean origin recap and sends
    // an unfiltered Done that prunes X's still-owned record.
    let config = json!({"inventory":{"hosts":{"a":{},"b":{}},"groups":{}},"plays":[
        {"targets":["a"],"tasks":[
            {"name":"delegated","operation":{"kind":"command","argv":["true"]},"delegateTo":"b","delegateAllow":["b"],"tags":["slow"]}
        ]},
        {"targets":["b"],"tasks":[
            {"name":"direct","operation":{"kind":"command","argv":["true"]},"tags":["fast"]}
        ]}
    ]});
    let mut backend = Fake::default();
    let options = Options {
        tags: vec!["fast".into()],
        ..Default::default()
    };
    let recaps = run(
        &config,
        &std::collections::BTreeMap::new(),
        &options,
        &mut backend,
        &mut std::collections::BTreeMap::new(),
    )
    .await?;
    assert!(recaps["a"].filtered, "the tag skip filters the origin");
    assert!(
        recaps["b"].filtered,
        "the destination inherits the origin's incompleteness"
    );
    assert!(
        backend
            .calls
            .iter()
            .all(|call| call.host != "b" || call.task.name == "direct"),
        "nothing was delegated this run: the mark comes from the play's literal target"
    );
    Ok(())
}

use super::*;

#[tokio::test]
async fn a_false_leaf_condition_marks_its_recap_filtered() -> Result<()> {
    // Thread 302: a `when:false` leaf omits a task the workflow keeps,
    // the same incomplete session a tag rejection is — the run's end
    // must expire nothing, or the agent prunes the records of every
    // task the condition skipped.
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"kept","operation":{"kind":"debug","msg":"kept"}},
        {"name":"gated","when":{"kind":"literal","value":false},"operation":{"kind":"debug","msg":"never"}}
    ]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 1);
    assert_eq!(recap["h"].skipped, 1);
    assert!(
        recap["h"].filtered,
        "a run that omits a condition-false task is filtered"
    );
    Ok(())
}

#[tokio::test]
async fn a_false_block_condition_marks_its_recap_filtered() -> Result<()> {
    // Thread 302, the block half: a `when:false` block omits every
    // task it keeps, so the omission latches where the block is
    // rejected — the leaves are never visited to latch it.
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"kept","operation":{"kind":"debug","msg":"kept"}},
        {"name":"gated block","when":{"kind":"literal","value":false},"block":[
            {"name":"inner","operation":{"kind":"debug","msg":"never"}}
        ]}
    ]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ok, 1);
    assert!(
        recap["h"].filtered,
        "a run that omits a condition-false block is filtered"
    );
    Ok(())
}

use super::*;

/// A change rule may fold in the change of an earlier task. The timer's
/// enablement task reports the change of the unit file beside its own, so
/// a watcher of the timer sees the file's change even when the
/// enablement itself did not drift.
#[tokio::test]
async fn a_change_rule_folds_in_an_earlier_tasks_change() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"write the unit file","register":"unit","operation":{"kind":"command","argv":["write"]}},
        {"name":"enable the unit","register":"timer","changedWhen":{"kind":"call","function":"or","args":[{"kind":"ref","path":["result","changed"]},{"kind":"ref","path":["results","unit","changed"]}]},"operation":{"kind":"command","argv":["enable"]}},
        {"name":"observe","operation":{"kind":"set_fact","values":{"seen":{"kind":"ref","path":["results","timer","changed"]}}}}
    ]}]});
    let mut backend = Fake {
        results: [
            json!({"changed":true,"failed":false}),
            json!({"changed":false,"failed":false}),
        ]
        .into(),
        ..Fake::default()
    };
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    // The enablement task changed nothing of its own, but the unit file
    // did: the step changed.
    assert_eq!(backend.events[1]["result"]["changed"], true);
    // A watcher of the step sees the change.
    assert_eq!(backend.events[2]["result"]["facts"]["seen"], true);
    // Both tasks that changed are counted: the file, and the enablement
    // that carries its change.
    assert_eq!(recap["h"].changed, 2);
    Ok(())
}

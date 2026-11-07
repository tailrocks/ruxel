use super::*;
#[tokio::test]
async fn skipped_block_reports_each_direct_child_and_continues() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"block","when":{"kind":"literal","value":false},"block":[
            {"name":"one","operation":{"kind":"fail","msg":"must not run"}},
            {"name":"two","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","missing"]}}}
        ],"rescue":[{"name":"rescue","operation":{"kind":"fail"}}],"always":[{"name":"always","operation":{"kind":"fail"}}]},
        {"name":"after","operation":{"kind":"debug","msg":"continued"}}
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
    assert_eq!(recap["h"].skipped, 2);
    assert_eq!(recap["h"].ok, 1);
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events.len(), 3);
    for (event, name) in backend.events.iter().take(2).zip(["one", "two"]) {
        assert_eq!(event["task"], name);
        assert_eq!(event["result"]["skipped"], true);
        assert_eq!(event["result"]["changed"], false);
    }
    assert_eq!(backend.events[2]["task"], "after");
    assert_eq!(backend.events[2]["result"]["msg"], "continued");
    assert!(backend.calls.is_empty());
    Ok(())
}
#[tokio::test]
async fn skipped_block_preserves_inherited_and_child_no_log() -> Result<()> {
    for (block_private, child_private) in [(false, true), (true, false)] {
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"block","noLog":block_private,"when":{"kind":"literal","value":false},"block":[{"name":"child","noLog":child_private,"operation":{"kind":"debug","msg":"synthetic"}}]}]}]});
        let mut backend = Fake::default();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recap["h"].skipped, 1);
        assert_eq!(backend.events[0]["result"]["censored"], true);
    }
    Ok(())
}

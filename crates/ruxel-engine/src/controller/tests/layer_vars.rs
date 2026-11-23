use super::*;

#[tokio::test]
async fn play_variables_use_same_layer_overrides_and_inherited_self() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{"vars":{"value":{"kind":"literal","value":"host"},"parent":{"kind":"literal","value":"inherited"}}}},"groups":{}},"plays":[{"targets":["h"],"vars":{"derived":{"kind":"ref","path":["vars","value"]},"self_ref":{"kind":"ref","path":["vars","parent"]},"value":{"kind":"literal","value":"play"},"parent":{"kind":"ref","path":["vars","parent"]}},"tasks":[{"name":"derived","operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","derived"]}]}}},{"name":"self","operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","self_ref"]}]}}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let params = |index: usize| -> Result<Value> {
        Ok(serde_json::from_slice(
            &backend.calls[index].task.iterations[0].params_json,
        )?)
    };
    assert_eq!(params(0)?["argv"][0], "play");
    assert_eq!(params(1)?["argv"][0], "inherited");
    Ok(())
}

#[tokio::test]
async fn persisted_set_fact_still_outranks_later_play_variables() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"operation":{"kind":"set_fact","values":{"value":{"kind":"literal","value":"fact"}}}}]},{"targets":["h"],"vars":{"derived":{"kind":"ref","path":["vars","value"]},"value":{"kind":"literal","value":"play"}},"tasks":[{"operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","derived"]}]}}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["argv"][0], "fact");
    Ok(())
}

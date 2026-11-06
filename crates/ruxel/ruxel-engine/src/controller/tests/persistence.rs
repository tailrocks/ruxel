use super::*;

#[tokio::test]
async fn set_facts_persist_across_plays_per_host() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"a":{},"b":{}},"groups":{}},"plays":[{"targets":["all"],"tasks":[{"name":"set","operation":{"kind":"set_fact","values":{"answer":{"kind":"ref","path":["inventoryHost"]}}}}]},{"targets":["all"],"tasks":[{"name":"use","operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","answer"]}}}]}]});
    let mut backend = Fake::default();
    let mut facts = BTreeMap::new();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut facts,
    )
    .await?;
    assert_eq!(backend.calls.len(), 2);
    for request in &backend.calls {
        let params: Value = serde_json::from_slice(&request.task.iterations[0].params_json)?;
        assert_eq!(params["cmd"], request.host);
    }
    assert_eq!(facts.len(), 2);
    Ok(())
}

#[tokio::test]
async fn sensitive_set_facts_remain_sensitive_across_plays() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"set","noLog":true,"operation":{"kind":"set_fact","values":{"answer":{"kind":"literal","value":"synthetic-private-marker"}}}}]},{"targets":["h"],"tasks":[{"name":"use","operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","answer"]}}}]}]});
    let mut backend = Fake::default();
    let mut facts = BTreeMap::new();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut facts,
    )
    .await?;
    assert!(facts["h"].is_secret());
    assert!(backend.calls[0].task.no_log);
    assert!(
        !backend
            .events
            .iter()
            .any(|event| event.to_string().contains("synthetic-private-marker"))
    );
    Ok(())
}

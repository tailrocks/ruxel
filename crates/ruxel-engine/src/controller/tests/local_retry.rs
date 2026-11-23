use super::*;

#[tokio::test]
async fn local_until_failure_is_counted_and_can_be_ignored() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","ignoreErrors":true,"retries":2,"delaySeconds":0,"until":{"kind":"literal","value":false},"operation":{"kind":"debug","msg":"public"}},{"name":"next","operation":{"kind":"debug","msg":"after"}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.events[0]["result"]["attempts"], 3);
    assert_eq!(backend.events[0]["result"]["failed"], true);
    assert_eq!(backend.events[1]["task"], "next");
    assert!(backend.events[1]["result"].get("attempts").is_none());
    assert_eq!(recaps["h"].ignored, 1);
    Ok(())
}

#[tokio::test]
async fn local_until_sees_registered_current_result_and_preserves_facts() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"fact","register":"answer","retries":3,"delaySeconds":0,"until":{"kind":"ref","path":["results","answer","facts","ready"]},"operation":{"kind":"set_fact","values":{"ready":{"kind":"literal","value":true}}}},{"name":"next","when":{"kind":"ref","path":["vars","ready"]},"operation":{"kind":"debug","msg":"after"}}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.events[0]["result"]["attempts"], 1);
    assert_eq!(backend.events[1]["task"], "next");
    assert_eq!(recap["h"].failed, 0);
    Ok(())
}

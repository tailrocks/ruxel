use super::*;

#[tokio::test]
async fn loop_summary_fields_are_available_in_registered_results() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"empty","register":"empty","loop":{"kind":"literal","value":[]},"operation":{"kind":"command","cmd":"unused"}},
        {"name":"complete","register":"complete","loop":{"kind":"literal","value":[1]},"operation":{"kind":"command","cmd":"ok"}},
        {"name":"skipped","register":"skipped","loop":{"kind":"literal","value":[1]},"when":false,"operation":{"kind":"command","cmd":"unused"}},
        {"name":"failure","register":"failure","loop":{"kind":"literal","value":[1]},"ignoreErrors":true,"operation":{"kind":"fail"}},
        {"name":"observe","operation":{"kind":"set_fact","values":{"empty":{"kind":"ref","path":["results","empty","skip_reason"]},"emptyAlias":{"kind":"ref","path":["results","empty","skipped_reason"]},"complete":{"kind":"ref","path":["results","complete","msg"]},"skipped":{"kind":"ref","path":["results","skipped","msg"]},"failure":{"kind":"ref","path":["results","failure","msg"]}}}}
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
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(recap["h"].ignored, 1);
    assert_eq!(
        backend.events[4]["result"]["facts"],
        json!({"empty":"No items in the list","emptyAlias":"No items in the list","complete":"All items completed","skipped":"All items skipped","failure":"One or more items failed"})
    );
    Ok(())
}

#[tokio::test]
async fn empty_loop_preserves_source_sensitivity_in_events_and_registered_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"public loop","loop":{"kind":"literal","value":[]},"operation":{"kind":"debug","msg":"unused"}},
        {"name":"private fact","noLog":true,"operation":{"kind":"set_fact","values":{"privateList":{"kind":"literal","value":[]}}}},
        {"name":"private loop","register":"privateLoop","loop":{"kind":"ref","path":["vars","privateList"]},"operation":{"kind":"debug","msg":"unused"}},
        {"name":"private result","operation":{"kind":"set_fact","values":{"reason":{"kind":"ref","path":["results","privateLoop","skip_reason"]}}}}
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
    assert_eq!(
        backend.events[0]["result"]["skip_reason"],
        "No items in the list"
    );
    assert_eq!(backend.events[2]["result"]["censored"], true);
    assert_eq!(backend.events[3]["result"]["censored"], true);
    Ok(())
}

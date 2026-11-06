use super::*;

#[tokio::test]
async fn task_variables_resolve_for_each_custom_item_with_dependencies() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"derived":{"kind":"literal","value":"inherited"}},"tasks":[{"name":"loop","loopVar":"entry","loop":{"kind":"ref","path":["vars","inputs"]},"vars":{"inputs":{"kind":"literal","value":["one","two"]},"alias":{"kind":"ref","path":["vars","derived"]},"derived":{"kind":"ref","path":["entry"]}},"operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","alias"]}}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let commands: Vec<_> = backend
        .calls
        .iter()
        .map(|call| {
            serde_json::from_slice::<Value>(&call.task.iterations[0].params_json)
                .map(|value| value["cmd"].clone())
        })
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(commands, [json!("one"), json!("two")]);
    Ok(())
}

#[tokio::test]
async fn empty_loop_does_not_resolve_item_variables() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"empty","loop":[],"vars":{"value":{"kind":"ref","path":["item"]}},"operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","value"]}}}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert!(backend.calls.is_empty());
    assert_eq!(recap["h"].skipped, 1);
    Ok(())
}

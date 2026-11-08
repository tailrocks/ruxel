use super::*;

#[tokio::test]
async fn retired_variables_name_is_a_custom_loop_binding() -> Result<()> {
    let config = crate::model::validate_config(&json!({
        "inventory":{"hosts":{"h":{"address":"fixture.invalid"}},"groups":{}},
        "plays":[{"name":"custom binding","targets":["h"],"vars":{"prefix":{"kind":"literal","value":"canonical"}},"tasks":[{
            "name":"loop","loopVar":"variables","loop":{"kind":"list","items":[
                {"kind":"literal","value":"one"},{"kind":"literal","value":"two"}
            ]},
            "operation":{"kind":"debug","msg":{"kind":"call","function":"concat","args":[
                {"kind":"ref","path":["vars","prefix"]},
                {"kind":"literal","value":"|"},
                {"kind":"ref","path":["variables"]},
                {"kind":"literal","value":"|"},
                {"kind":"ref","path":["item"]}
            ]}}
        }]}]
    }))?;
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
    assert_eq!(
        backend.events[0]["result"]["results"][0]["msg"],
        "canonical|one|one"
    );
    assert_eq!(
        backend.events[0]["result"]["results"][1]["msg"],
        "canonical|two|two"
    );
    Ok(())
}

#[tokio::test]
async fn task_variables_resolve_for_each_custom_item_with_dependencies() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"derived":{"kind":"literal","value":"inherited"}},"tasks":[{"name":"loop","loopVar":"entry","loop":{"kind":"ref","path":["vars","inputs"]},"vars":{"inputs":{"kind":"literal","value":["one","two"]},"alias":{"kind":"ref","path":["vars","derived"]},"derived":{"kind":"ref","path":["entry"]}},"operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","alias"]}]}}}]}]});
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
                .map(|value| value["argv"][0].clone())
        })
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(commands, [json!("one"), json!("two")]);
    Ok(())
}

#[tokio::test]
async fn empty_loop_does_not_resolve_item_variables() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"empty","loop":[],"vars":{"value":{"kind":"ref","path":["item"]}},"operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","value"]}]}}}]}]});
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

#[tokio::test]
async fn loop_variable_override_can_reference_inherited_value() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"name":{"kind":"literal","value":"parent"}},"tasks":[{"loop":["one"],"vars":{"name":{"kind":"ref","path":["vars","name"]}},"operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","name"]}]}}}]}]});
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
    assert_eq!(params["argv"][0], "parent");
    Ok(())
}

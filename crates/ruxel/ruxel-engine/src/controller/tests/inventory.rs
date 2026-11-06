use super::*;

#[tokio::test]
async fn child_group_variables_override_parents_regardless_of_names() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{"a-child":{"hosts":["h"],"vars":{"value":{"kind":"literal","value":"child"}}},"z-parent":{"children":["a-child"],"vars":{"value":{"kind":"literal","value":"parent"}}}}},"plays":[{"targets":["all"],"tasks":[{"name":"test","operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","value"]}}}]}]});
    let mut backend = Fake::default();
    run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "child");
    Ok(())
}

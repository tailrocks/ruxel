use super::*;

#[tokio::test]
async fn plan_censors_lookup_nodes_in_tasks_blocks_and_handlers() -> Result<()> {
    let lookup = json!({"kind":"lookup","provider":"synthetic","key":"fixture-sensitive-value","argv":["fixture-sensitive-argument"]});
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","vars":{"private":lookup},"block":[{"name":"nested","operation":{"kind":"debug","msg":{"kind":"call","function":"concat","args":[{"kind":"literal","value":"public"},lookup]}}}]}],"handlers":[{"name":"handler","operation":{"kind":"debug","msg":lookup}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Plan,
        ..Options::default()
    };
    run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let events = serde_json::to_string(&backend.events)?;
    assert!(!events.contains("fixture-sensitive"));
    assert!(events.contains("public"));
    assert_eq!(
        backend.events[0]["task"]["vars"]["private"],
        json!({"kind":"lookup","censored":true})
    );
    assert_eq!(
        backend.events[1]["task"]["operation"]["msg"],
        json!({"kind":"lookup","censored":true})
    );
    assert!(backend.calls.is_empty());
    Ok(())
}

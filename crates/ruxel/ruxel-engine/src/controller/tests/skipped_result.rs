use super::*;

#[tokio::test]
async fn skipped_task_and_loop_items_have_usable_failure_status() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"conditional","when":false,"register":"conditional","operation":{"kind":"debug","msg":"unused"}},
        {"name":"loop","loop":{"kind":"literal","value":["a","b"]},"when":false,"operation":{"kind":"debug","msg":"unused"}},
        {"name":"observe","operation":{"kind":"set_fact","values":{"failed":{"kind":"ref","path":["results","conditional","failed"]},"reason":{"kind":"ref","path":["results","conditional","skip_reason"]}}}}
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
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events[0]["result"]["failed"], false);
    assert_eq!(backend.events[2]["result"]["facts"]["failed"], false);
    assert_eq!(
        backend.events[2]["result"]["facts"]["reason"],
        "Conditional result was False"
    );
    for result in backend.events[1]["result"]["results"]
        .as_array()
        .context("loop results")?
    {
        assert_eq!(result["failed"], false);
        assert_eq!(result["skip_reason"], "Conditional result was False");
    }
    Ok(())
}

#[tokio::test]
async fn private_skipped_results_remain_private_when_registered() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"public","when":false,"operation":{"kind":"debug"}},
        {"name":"private","noLog":true,"when":false,"register":"private","operation":{"kind":"debug"}},
        {"name":"observe private","operation":{"kind":"set_fact","values":{"reason":{"kind":"ref","path":["results","private","skip_reason"]}}}},
        {"name":"private block","noLog":true,"block":[{"name":"private loop","loop":{"kind":"literal","value":["fixture"]},"when":false,"register":"privateLoop","operation":{"kind":"debug"}}]},
        {"name":"observe loop","operation":{"kind":"set_fact","values":{"summary":{"kind":"ref","path":["results","privateLoop","msg"]}}}}
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
    assert_eq!(recap["h"].skipped, 3);
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events[0]["result"]["skipped"], true);
    for event in &backend.events[1..] {
        assert_eq!(event["result"]["censored"], true);
    }
    Ok(())
}

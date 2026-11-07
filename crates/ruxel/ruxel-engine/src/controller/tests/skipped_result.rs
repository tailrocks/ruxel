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

use super::*;

#[tokio::test]
async fn successful_local_results_expose_failure_status_when_registered() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"debug","register":"debug","operation":{"kind":"debug","msg":"fixture"}},
        {"name":"fact","register":"fact","operation":{"kind":"set_fact","values":{"fixture":true}}},
        {"name":"pause","register":"pause","checkMode":true,"operation":{"kind":"pause"}},
        {"name":"observe","operation":{"kind":"set_fact","values":{"debugFailed":{"kind":"ref","path":["results","debug","failed"]},"factFailed":{"kind":"ref","path":["results","fact","failed"]},"pauseFailed":{"kind":"ref","path":["results","pause","failed"]}}}}
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
    assert_eq!(backend.events[2]["result"]["skipped"], true);
    assert_eq!(
        backend.events[3]["result"]["facts"],
        json!({"debugFailed":false,"factFailed":false,"pauseFailed":false})
    );
    Ok(())
}

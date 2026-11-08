use super::*;

#[tokio::test]
async fn result_conditions_publish_outcomes_and_use_original_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"overrides","register":"saved","operation":{"kind":"command","argv":["synthetic"]},"failedWhen":true,"changedWhen":{"kind":"ref","path":["result","failed"]},"ignoreErrors":true},
        {"name":"metadata","operation":{"kind":"set_fact","values":{"failedOutcome":{"kind":"ref","path":["results","saved","failed_when_result"]},"changedOutcome":{"kind":"ref","path":["results","saved","changed_when_result"]}}}}
    ]}]});
    let mut backend = Fake {
        results: [json!({"changed":true,"failed":false})].into(),
        ..Fake::default()
    };
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ignored, 1);
    assert_eq!(backend.events[0]["result"]["failed_when_result"], true);
    assert_eq!(backend.events[0]["result"]["changed_when_result"], false);
    assert_eq!(backend.events[0]["result"]["changed"], false);
    assert_eq!(backend.events[1]["result"]["facts"]["failedOutcome"], true);
    assert_eq!(
        backend.events[1]["result"]["facts"]["changedOutcome"],
        false
    );
    Ok(())
}

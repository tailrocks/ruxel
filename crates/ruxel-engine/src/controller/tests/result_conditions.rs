use super::*;

#[tokio::test]
async fn result_conditions_publish_outcomes_and_use_original_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"overrides","register":"saved","operation":{"kind":"command","argv":["synthetic"]},"failedWhen":true,"changedWhen":{"kind":"ref","path":["result","failed"]},"ignoreErrors":true},
        {"name":"metadata","operation":{"kind":"set_fact","values":{"failedOutcome":{"kind":"ref","path":["results","saved","failed"]},"changedOutcome":{"kind":"ref","path":["results","saved","changed"]}}}}
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
    assert_eq!(backend.events[0]["result"]["failed"], true);
    assert_eq!(backend.events[0]["result"]["changed"], false);
    assert!(
        backend.events[0]["result"]
            .get("failed_when_result")
            .is_none()
    );
    assert!(
        backend.events[0]["result"]
            .get("changed_when_result")
            .is_none()
    );
    assert_eq!(backend.events[1]["result"]["facts"]["failedOutcome"], true);
    assert_eq!(
        backend.events[1]["result"]["facts"]["changedOutcome"],
        false
    );
    Ok(())
}

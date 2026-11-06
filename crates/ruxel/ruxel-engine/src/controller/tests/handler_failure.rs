use super::*;

#[tokio::test]
async fn failing_handler_keeps_failure_without_misreporting_pending_handler() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"trigger","notify":["first","second"],"operation":{"kind":"command","cmd":"change"}}],"handlers":[{"name":"first","operation":{"kind":"fail","msg":"expected"}},{"name":"second","operation":{"kind":"command","cmd":"must not run"}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    assert_eq!(recaps["h"].failed, 1);
    assert_eq!(recaps["h"].changed, 1);
    assert_eq!(backend.events[1]["task"], "first");
    Ok(())
}

#[tokio::test]
async fn unknown_notification_remains_an_error_after_handler_failure() {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"notify":["first","second","missing"],"operation":{"kind":"command","cmd":"change"}}],"handlers":[{"name":"first","operation":{"kind":"fail"}},{"name":"second","operation":{"kind":"debug"}}]}]});
    let mut backend = Fake::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await;
    assert!(error.is_err_and(|error| error.to_string().contains("missing")));
}

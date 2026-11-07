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

#[tokio::test]
async fn backward_notification_does_not_rerun_or_report_known_handler() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"trigger","notify":["first","second"],"operation":{"kind":"command","cmd":"trigger"}}],"handlers":[{"name":"first","operation":{"kind":"command","cmd":"first"}},{"name":"second","notify":["first"],"operation":{"kind":"command","cmd":"second"}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let names: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.task.name.as_str())
        .collect();
    assert_eq!(names, ["trigger", "first", "second"]);
    assert_eq!(recaps["h"].failed, 0);
    Ok(())
}

#[tokio::test]
async fn backward_notification_does_not_hide_unknown_handler() {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"notify":["first","second"],"operation":{"kind":"command","cmd":"trigger"}}],"handlers":[{"name":"first","operation":{"kind":"command","cmd":"first"}},{"name":"second","notify":["first","missing"],"operation":{"kind":"command","cmd":"second"}}]}]});
    let mut backend = Fake::default();
    let result = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await;
    assert!(result.is_err_and(|error| error.to_string().contains("missing")));
}

#[tokio::test]
async fn later_handler_queues_earlier_handler_once_even_with_a_cycle() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"trigger","notify":["second"],"operation":{"kind":"command","cmd":"trigger"}}],"handlers":[{"name":"first","notify":["second"],"operation":{"kind":"command","cmd":"first"}},{"name":"second","notify":["first"],"operation":{"kind":"command","cmd":"second"}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let names: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.task.name.as_str())
        .collect();
    assert_eq!(names, ["trigger", "second", "first"]);
    assert_eq!(recaps["h"].failed, 0);
    Ok(())
}

#[tokio::test]
async fn backward_handler_failure_stops_pending_known_handlers() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"trigger","notify":["second"],"operation":{"kind":"command","cmd":"trigger"}}],"handlers":[{"name":"first","operation":{"kind":"fail","msg":"expected"}},{"name":"third","operation":{"kind":"command","cmd":"must not run"}},{"name":"second","notify":["first","third"],"operation":{"kind":"command","cmd":"second"}}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let names: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.task.name.as_str())
        .collect();
    assert_eq!(names, ["trigger", "second"]);
    assert_eq!(recap["h"].failed, 1);
    assert_eq!(
        backend.events.last().context("handler failure event")?["task"],
        "first"
    );
    Ok(())
}

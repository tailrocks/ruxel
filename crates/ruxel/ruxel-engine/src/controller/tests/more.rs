use super::*;

#[test]
fn group_cycles_are_rejected() -> Result<()> {
    let groups = json!({"a":{"children":["b"]},"b":{"children":["a"]}});
    assert!(
        group_hosts(
            "a",
            groups.as_object().context("groups")?,
            &mut BTreeSet::new()
        )
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn retry_and_result_conditions_see_current_registered_iteration() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","register":"attempt","retries":2,"delaySeconds":0,"until":{"kind":"ref","path":["results","attempt","ready"]},"failedWhen":{"kind":"ref","path":["results","attempt","bad"]},"changedWhen":{"kind":"ref","path":["results","attempt","changed"]},"operation":{"kind":"command","argv":["test"]}}]}]});
    let mut backend = Fake {
        results: [
            json!({"ready":false,"bad":true,"changed":false}),
            json!({"ready":true,"bad":false,"changed":true}),
        ]
        .into(),
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
    assert_eq!(backend.calls.len(), 2);
    assert_eq!(recap["h"].changed, 1);
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events[0]["result"]["attempts"], 2);
    Ok(())
}

#[tokio::test]
async fn earlier_retry_change_notifies_handler_and_keeps_final_fields() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","register":"attempt","retries":2,"delaySeconds":0,"until":{"kind":"ref","path":["result","ready"]},"notify":["handler"],"operation":{"kind":"command","argv":["test"]}}],"handlers":[{"name":"handler","operation":{"kind":"debug","msg":"done"}}]}]});
    let mut backend = Fake {
        results: [
            json!({"ready":false,"changed":true,"detail":"first"}),
            json!({"ready":true,"changed":false,"detail":"last"}),
        ]
        .into(),
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
    assert_eq!(recap["h"].changed, 1);
    assert_eq!(backend.calls.len(), 2);
    assert_eq!(backend.events[0]["result"]["detail"], "last");
    assert_eq!(backend.events[0]["result"]["changed"], true);
    assert_eq!(backend.events[1]["task"], "handler");
    Ok(())
}

#[tokio::test]
async fn exhausted_until_reports_attempt_count() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","retries":1,"delaySeconds":0,"until":{"kind":"ref","path":["result","ready"]},"operation":{"kind":"command","argv":["test"]}}]}]});
    let mut backend = Fake {
        results: [json!({"ready":false}), json!({"ready":false})].into(),
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
    assert_eq!(recap["h"].failed, 1);
    assert_eq!(backend.events[0]["result"]["attempts"], 2);
    assert_eq!(backend.calls.len(), 2);
    Ok(())
}

#[tokio::test]
async fn tagged_trigger_runs_untagged_handler() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"trigger","tags":["chosen"],"notify":["handler"],"operation":{"kind":"command","argv":["trigger"]}},{"name":"always","tags":["always"],"operation":{"kind":"command","argv":["always"]}},{"name":"excluded","operation":{"kind":"command","argv":["excluded"]}}],"handlers":[{"name":"handler","operation":{"kind":"command","argv":["handler"]}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        tags: vec!["chosen".into()],
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
    let names: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.task.name.as_str())
        .collect();
    assert_eq!(names, ["trigger", "handler"]);
    Ok(())
}

use super::*;

#[tokio::test]
async fn nested_targets_loop_and_registered_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"host":{}},"groups":{"outer":{"children":["inner"]},"inner":{"hosts":["host"]}}},"plays":[{"targets":["outer"],"tasks":[{"name":"loop","loop":["a","b"],"operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["item"]}]}},"register":"first"},{"name":"next","when":{"kind":"ref","path":["results","first","changed"]},"operation":{"kind":"command","argv":["next"]}}]}]});
    let mut backend = Fake::default();
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 3);
    assert_eq!(recaps["host"].changed, 2);
    assert_eq!(
        serde_json::from_slice::<Value>(&backend.calls[0].task.iterations[0].params_json)?["argv"]
            [0],
        "a"
    );
    Ok(())
}

#[tokio::test]
async fn rescue_always_and_handler_run_in_order() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","block":[{"name":"bad","operation":{"kind":"fail"}}],"rescue":[{"name":"recovered","operation":{"kind":"command","argv":["rescue"]},"notify":["handler"]}],"always":[{"name":"cleanup","operation":{"kind":"command","argv":["always"]}}]}],"handlers":[{"name":"handler","operation":{"kind":"command","argv":["handler"]}}]}]});
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
    assert_eq!(names, ["recovered", "cleanup", "handler"]);
    assert_eq!(recap["h"].rescued, 1);
    assert_eq!(recap["h"].failed, 0);
    Ok(())
}

#[tokio::test]
async fn no_log_censors_events_and_diff_checks() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"secret","noLog":true,"operation":{"kind":"command","argv":["private"]}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Diff,
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
    assert!(backend.calls[0].task.force_check_mode);
    assert_eq!(backend.events[0]["result"]["censored"], true);
    assert!(!backend.events[0].to_string().contains("private"));
    Ok(())
}

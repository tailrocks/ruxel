use super::*;

#[tokio::test]
async fn conservative_plan_censors_tasks_and_handlers() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"synthetic-marker","operation":{"kind":"debug","msg":"synthetic-marker"}}],"handlers":[{"name":"synthetic-marker","operation":{"kind":"debug","msg":"synthetic-marker"}}]}]});
    let mut backend = Fake {
        conservative_sensitive: true,
        ..Default::default()
    };
    let options = Options {
        mode: Mode::Plan,
        ..Default::default()
    };
    let recap = run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].planned, 1);
    assert_eq!(backend.events.len(), 2);
    assert!(
        backend
            .events
            .iter()
            .all(|event| event["host"] == "[censored]")
    );
    assert!(
        backend
            .events
            .iter()
            .all(|event| event["task"] == json!({"censored":true}))
    );
    assert!(!serde_json::to_string(&backend.events)?.contains("synthetic-marker"));
    Ok(())
}

/// The conservative projection censors the task, but the marker the
/// controller builds itself — the known-after-apply marker of an apply-only
/// step — is safe metadata: the plan says that the step happens at apply,
/// and nothing else about it.
#[tokio::test]
async fn the_conservative_plan_keeps_the_known_after_apply_marker() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    let mut backend = Fake {
        conservative_sensitive: true,
        ..Default::default()
    };
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
    assert_eq!(backend.events.len(), 1);
    assert_eq!(backend.events[0]["host"], "[censored]");
    assert_eq!(
        backend.events[0]["task"],
        json!({"censored":true,"knownAfterApply":true})
    );
    Ok(())
}

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

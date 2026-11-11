use super::*;

/// `FetchSecret` is apply-only: in check and diff the controller answers
/// known-after-apply without sending anything to the host and without
/// contacting fnox; in apply the task runs on the one named host and the
/// controller stores the content; in plan the planned task marks itself
/// known-after-apply too.
#[tokio::test]
async fn fetch_secret_is_known_after_apply_in_check_and_diff() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    for mode in [Mode::Check, Mode::Diff] {
        let mut backend = Fake::default();
        let options = Options {
            mode,
            ..Options::default()
        };
        let recap = run(
            &config,
            &BTreeMap::new(),
            &options,
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert!(
            backend.calls.is_empty(),
            "{mode:?}: the host is not contacted",
        );
        assert_eq!(recap["h"].ok, 1);
        let events: Vec<_> = backend
            .events
            .iter()
            .filter(|event| event.get("task").is_some())
            .collect();
        assert_eq!(events.len(), 1, "{mode:?}: one task event");
        assert_eq!(events[0]["result"]["knownAfterApply"], json!(true));
        assert_eq!(events[0]["result"]["changed"], json!(false));
        assert_eq!(events[0]["result"]["censored"], json!(true));
    }
    Ok(())
}

#[tokio::test]
async fn fetch_secret_runs_only_in_apply_and_marks_the_plan() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1, "apply sends the task once");
    assert_eq!(backend.calls[0].task.module, "fetchSecret");
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["path"], "/etc/kubernetes/admin.conf");
    assert_eq!(params["key"], "K8S_ADMIN_KUBECONFIG");
    assert!(backend.calls[0].task.no_log, "a fetched secret is no-log");
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
    assert!(backend.calls.is_empty(), "plan never executes");
    assert_eq!(backend.events[0]["task"]["knownAfterApply"], json!(true));
    assert_eq!(backend.events[0]["task"]["censored"], json!(true));
    Ok(())
}

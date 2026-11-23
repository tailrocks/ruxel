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

/// A failed fetch is a failed task, not a controller error: the run
/// reports it the way every other failed task is reported, so the recap
/// counts it and `ignoreErrors` ignores it.
#[tokio::test]
async fn a_failed_fetch_is_counted_as_a_failed_task() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    let mut backend = Fake::default();
    backend
        .results
        .push_back(json!({"failed": true, "changed": false}));
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].failed, 1, "the failed fetch is counted");
    assert_eq!(recap["h"].ok, 0);
    // The same failure with `ignoreErrors` is ignored, not failed.
    let mut backend = Fake::default();
    backend
        .results
        .push_back(json!({"failed": true, "changed": false}));
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"ignoreErrors":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ignored, 1, "the failure is ignored");
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(recap["h"].ok, 1);
    Ok(())
}

/// The fingerprints and diff sentences a no-log result shows are the
/// controller's own: exactly eight lowercase hex, and the sentence built
/// over them, whole. A result that carries any other fingerprint, or a
/// diff that merely starts like the sentence and carries material after
/// it — an agent can ship either in its result — is censored: agent
/// material reaches no output.
#[tokio::test]
async fn a_no_log_result_shows_only_controller_built_fingerprints_and_diffs() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    // The controller's own shape passes: eight lowercase hex, and the
    // sentence built over them.
    let mut backend = Fake::default();
    backend.results.push_back(json!({
        "changed":true,
        "fingerprint":"abcd1234",
        "diff":"stored in fnox (fingerprint abcd1234)"
    }));
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(
        backend.events[0]["result"]["fingerprint"],
        json!("abcd1234")
    );
    assert_eq!(
        backend.events[0]["result"]["diff"],
        json!("stored in fnox (fingerprint abcd1234)")
    );
    // An agent-supplied fingerprint, and a diff that starts like the
    // sentence and carries material after it, reach no output.
    let mut backend = Fake::default();
    backend.results.push_back(json!({
        "changed":true,
        "fingerprint":"synthetic-agent-fingerprint",
        "diff":"stored in fnox (fingerprint abcd1234) synthetic-agent-material"
    }));
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let visible = serde_json::to_string(&backend.events[0]["result"])?;
    assert!(!visible.contains("synthetic-agent"), "{visible}");
    assert!(
        backend.events[0]["result"].get("fingerprint").is_none(),
        "an agent-supplied fingerprint passed: {visible}"
    );
    assert!(
        backend.events[0]["result"].get("diff").is_none(),
        "an agent-supplied diff passed: {visible}"
    );
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

/// A no-log result whose diff carries the controller's exact shape —
/// `73656372` is the hex of `secr`, four bytes of secret in the
/// fingerprint's own shape — keeps no diff when no controller fingerprint
/// stands behind it: shape does not establish who built a value, so the
/// projection keeps a sentence only as the controller built it, over the
/// fingerprint the controller set beside it.
#[tokio::test]
async fn a_well_shaped_agent_diff_without_a_fingerprint_reaches_no_output() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"Store the cluster admin kubeconfig","noLog":true,"operation":{"kind":"fetchSecret","path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}}]}]});
    let mut backend = Fake::default();
    backend.results.push_back(json!({
        "changed": true,
        "diff": "content changed (fingerprint 73656372)"
    }));
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let visible = serde_json::to_string(&backend.events[0]["result"])?;
    assert!(
        backend.events[0]["result"].get("diff").is_none(),
        "a well-shaped agent diff passed: {visible}"
    );
    assert!(
        !visible.contains("73656372"),
        "four bytes of agent material reached the output: {visible}"
    );
    Ok(())
}

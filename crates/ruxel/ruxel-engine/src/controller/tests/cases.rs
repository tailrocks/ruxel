use super::*;
#[tokio::test]
async fn until_uses_result_and_loop_label_uses_item() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","loop":["one"],"loopLabel":{"kind":"ref","path":["item"]},"retries":2,"until":{"kind":"ref","path":["result","ready"]},"operation":{"kind":"command","cmd":"test"}}]}]});
    let mut backend = Fake {
        results: [json!({"ready":false}), json!({"ready":true,"changed":true})].into(),
        ..Fake::default()
    };
    let recap = run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    assert_eq!(backend.calls.len(), 2);
    assert_eq!(backend.calls[0].task.iterations[0].item_label, "one");
    assert_eq!(recap["h"].changed, 1);
    assert_eq!(recap["h"].failed, 0);
    Ok(())
}

#[tokio::test]
async fn result_conditions_override_agent_status() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"override","changedWhen":{"kind":"literal","value":false},"failedWhen":{"kind":"literal","value":false},"operation":{"kind":"command","cmd":"test"}}]}]});
    let mut backend = Fake {
        results: [json!({"failed":true,"changed":true})].into(),
        ..Fake::default()
    };
    let recap = run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(recap["h"].changed, 0);
    assert_eq!(recap["h"].ok, 1);
    Ok(())
}

#[tokio::test]
async fn plan_keeps_runtime_expressions_deferred_and_censors_nested_secrets() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","block":[{"name":"private","noLog":true,"operation":{"kind":"command","cmd":"private"}},{"name":"deferred","when":{"kind":"ref","path":["results","notYetAvailable"]},"operation":{"kind":"command","cmd":"test"}}]}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Plan,
        ..Options::default()
    };
    run(&config, &BTreeMap::new(), &options, &mut backend).await?;
    assert!(backend.calls.is_empty());
    assert!(!backend.events[0].to_string().contains("private\"}"));
    assert_eq!(backend.events[0]["task"]["block"][0]["censored"], true);
    assert_eq!(backend.events[0]["task"]["block"][1]["when"]["kind"], "ref");
    Ok(())
}

#[tokio::test]
async fn set_fact_is_available_to_task_variables() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"set","operation":{"kind":"set_fact","values":{"answer":{"kind":"literal","value":"ready"}}}},{"name":"use","vars":{"derived":{"kind":"ref","path":["vars","answer"]}},"operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","derived"]}}}]}]});
    let mut backend = Fake::default();
    run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "ready");
    Ok(())
}

#[tokio::test]
async fn play_variables_resolve_forward_references() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"first":{"kind":"ref","path":["vars","second"]},"second":{"kind":"literal","value":"ready"}},"tasks":[{"name":"use","operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","first"]}}}]}]});
    let mut backend = Fake::default();
    run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "ready");
    Ok(())
}

#[tokio::test]
async fn check_mode_skips_pause() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"pause","operation":{"kind":"pause","seconds":30}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Check,
        ..Options::default()
    };
    let recap = run(&config, &BTreeMap::new(), &options, &mut backend).await?;
    assert!(backend.calls.is_empty());
    assert_eq!(recap["h"].skipped, 1);
    Ok(())
}

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
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"retry","register":"attempt","retries":2,"until":{"kind":"ref","path":["results","attempt","ready"]},"failedWhen":{"kind":"ref","path":["results","attempt","bad"]},"changedWhen":{"kind":"ref","path":["results","attempt","changed"]},"operation":{"kind":"command","cmd":"test"}}]}]});
    let mut backend = Fake {
        results: [
            json!({"ready":false,"bad":true,"changed":false}),
            json!({"ready":true,"bad":false,"changed":true}),
        ]
        .into(),
        ..Fake::default()
    };
    let recap = run(&config, &BTreeMap::new(), &Options::default(), &mut backend).await?;
    assert_eq!(backend.calls.len(), 2);
    assert_eq!(recap["h"].changed, 1);
    assert_eq!(recap["h"].failed, 0);
    Ok(())
}

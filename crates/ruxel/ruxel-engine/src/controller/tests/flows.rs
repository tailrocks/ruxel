use super::*;

#[tokio::test]
async fn retry_delegation_and_unrescued_always() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{},"other":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","block":[{"name":"retry","delegateTo":"other","retries":2,"operation":{"kind":"command","cmd":"try"}}],"always":[{"name":"cleanup","operation":{"kind":"shell","cmd":"cleanup"}}]},{"name":"unreachable","operation":{"kind":"command","cmd":"never"}}]}]});
    let mut backend = Fake {
        results: [
            json!({"failed":true}),
            json!({"failed":true}),
            json!({"changed":false}),
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
    assert_eq!(backend.calls.len(), 3);
    assert_eq!(backend.calls[0].host, "other");
    assert_eq!(backend.calls[1].host, "other");
    assert_eq!(backend.calls[2].task.name, "cleanup");
    assert_eq!(backend.calls[2].task.iterations[0].free_form, "cleanup");
    assert_eq!(recap["h"].failed, 1);
    Ok(())
}

#[tokio::test]
async fn ignored_failure_continues_and_block_censors_children() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"outer","noLog":true,"block":[{"name":"bad","operation":{"kind":"fail","msg":"private"},"ignoreErrors":true},{"name":"next","operation":{"kind":"command","cmd":"private"}}]}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].ignored, 1);
    assert_eq!(backend.calls.len(), 1);
    assert!(backend.calls[0].task.no_log);
    assert!(
        !backend
            .events
            .iter()
            .any(|event| event.to_string().contains("private"))
    );
    Ok(())
}

use super::*;

#[tokio::test]
async fn task_error_preserves_completed_recap() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"first","operation":{"kind":"command","cmd":"true"}},{"name":"bad","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","missing"]}}}]}]});
    let mut backend = Fake::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await
    .err()
    .context("run should fail")?;
    let partial = error
        .downcast_ref::<RunFailure>()
        .context("missing partial recap")?;
    assert_eq!(partial.recaps["h"].ok, 1);
    assert_eq!(partial.recaps["h"].changed, 1);
    assert_eq!(backend.calls.len(), 1);
    Ok(())
}

#[tokio::test]
async fn sensitive_task_error_does_not_expose_fact_value() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"secret","noLog":true,"operation":{"kind":"set_fact","values":{"private":{"kind":"literal","value":"synthetic-private-marker"}}}},{"name":"bad","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","private","missing"]}}}]}]});
    let mut backend = Fake::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await
    .err()
    .context("run should fail")?;
    assert!(!error.to_string().contains("synthetic-private-marker"));
    assert_eq!(
        error
            .downcast_ref::<RunFailure>()
            .context("missing partial recap")?
            .recaps["h"]
            .ok,
        1
    );
    Ok(())
}

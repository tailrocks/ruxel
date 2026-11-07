use super::*;

#[tokio::test]
async fn unknown_notifications_fail_before_any_effect() {
    for phase in ["preTasks", "tasks", "handlers"] {
        for section in ["block", "rescue", "always"] {
            let mut nested = json!({"block":[{"operation":{"kind":"debug"}}]});
            nested[section] =
                json!([{"when":false,"notify":["missing"],"operation":{"kind":"debug"}}]);
            let mut config = json!({"inventory":{"hosts":{"synthetic":{}}},"plays":[{"targets":["synthetic"],"tasks":[{"operation":{"kind":"command","cmd":"must not run"}}]}]});
            config["plays"][0][phase] = json!([nested]);
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
            assert!(backend.calls.is_empty());
            assert_eq!(backend.events, [] as [Value; 0]);
        }
    }
}

#[tokio::test]
async fn dynamic_notifications_are_rejected_without_evaluation() {
    let config = json!({"inventory":{"hosts":{"synthetic":{}}},"plays":[{"targets":["synthetic"],"tasks":[{"notify":[{"kind":"lookup","provider":"unexpected"}],"operation":{"kind":"command","cmd":"must not run"}}]}]});
    let mut backend = Fake::default();
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await;
    assert!(error.is_err_and(|error| error.to_string().contains("literal handler names")));
    assert!(backend.calls.is_empty());
    assert_eq!(backend.events, [] as [Value; 0]);
}

use super::*;

#[tokio::test]
async fn invalid_block_controls_fail_before_execution() -> Result<()> {
    for (field, value) in [("register", json!("result")), ("ignoreErrors", json!(true))] {
        for phase in ["preTasks", "tasks", "handlers"] {
            for section in ["block", "rescue", "always"] {
                let mut invalid =
                    json!({"name":"invalid", "block":[{"operation":{"kind":"debug"}}]});
                invalid[field] = value.clone();
                let mut parent = json!({"block":[{"operation":{"kind":"debug"}}]});
                parent[section] = json!([invalid]);
                let mut config = json!({"inventory":{"hosts":{"synthetic":{}}},"plays":[{"targets":["synthetic"],"tasks":[{"operation":{"kind":"debug"}}]}]});
                config["plays"][0][phase] = json!([parent]);
                let mut backend = Fake::default();
                let error = run(
                    &config,
                    &BTreeMap::new(),
                    &Options::default(),
                    &mut backend,
                    &mut BTreeMap::new(),
                )
                .await
                .expect_err("invalid block control");
                assert!(error.to_string().contains(field));
                assert!(backend.calls.is_empty());
                assert_eq!(backend.events, [] as [Value; 0]);
            }
        }
    }
    Ok(())
}

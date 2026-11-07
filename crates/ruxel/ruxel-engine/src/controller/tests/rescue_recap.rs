use super::*;
#[tokio::test]
async fn rescue_accounting_matches_failed_and_successful_rescue() -> Result<()> {
    for succeeds in [false, true] {
        let rescue = if succeeds {
            json!({"kind":"debug","msg":"rescued"})
        } else {
            json!({"kind":"fail","msg":"rescue failed"})
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
            {"name":"block","block":[{"name":"original","operation":{"kind":"fail","msg":"original failure"}}],"rescue":[{"name":"rescue","operation":rescue}],"always":[{"name":"always","operation":{"kind":"debug","msg":"cleanup"}}]},
            {"name":"after","operation":{"kind":"debug","msg":"continued"}}
        ]}]});
        let mut backend = Fake::default();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recap["h"].rescued, 1);
        assert_eq!(recap["h"].failed, u64::from(!succeeds));
        let names: Vec<_> = backend
            .events
            .iter()
            .filter_map(|event| event["task"].as_str())
            .collect();
        assert_eq!(
            names,
            if succeeds {
                vec!["original", "rescue", "always", "after"]
            } else {
                vec!["original", "rescue", "always"]
            }
        );
    }
    Ok(())
}
#[tokio::test]
async fn ignored_rescue_failure_does_not_fail_host() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"block","block":[{"name":"original","operation":{"kind":"fail"}}],"rescue":[{"name":"rescue","ignoreErrors":true,"operation":{"kind":"fail"}}]}]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(recap["h"].rescued, 1);
    assert_eq!(recap["h"].ignored, 1);
    Ok(())
}

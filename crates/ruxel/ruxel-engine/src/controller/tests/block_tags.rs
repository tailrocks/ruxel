use super::*;

#[tokio::test]
async fn tagged_descendants_survive_untagged_blocks_in_plan_and_apply() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"outer","block":[{"name":"inner","block":[
            {"name":"chosen","tags":["chosen"],"operation":{"kind":"debug","msg":"selected"}},
            {"name":"excluded","operation":{"kind":"fail","msg":"not selected"}}
        ]}]},
        {"name":"excluded block","when":{"kind":"ref","path":["vars","missing"]},"block":[{"name":"other","tags":["other"],"operation":{"kind":"fail"}}]},
        {"name":"inherited","tags":["chosen"],"block":[{"name":"inherited child","operation":{"kind":"debug","msg":"inherited"}}]},
        {"name":"always outer","block":[{"name":"always child","tags":["always"],"operation":{"kind":"debug","msg":"always"}}]}
    ]}]});
    for mode in [Mode::Apply, Mode::Plan] {
        let mut backend = Fake::default();
        let options = Options {
            mode,
            tags: vec!["chosen".into()],
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
        let events = serde_json::to_string(&backend.events)?;
        assert!(events.contains("chosen"));
        assert!(events.contains("inherited child"));
        assert!(events.contains("always child"));
        assert!(!events.contains("excluded block"));
        assert!(!events.contains("not selected"));
        if mode == Mode::Plan {
            assert_eq!(recap["h"].planned, 3);
        } else {
            assert_eq!(recap["h"].ok, 3);
            assert_eq!(recap["h"].failed, 0);
        }
    }
    Ok(())
}

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
        assert!(!events.contains("always child"));
        assert!(!events.contains("excluded block"));
        assert!(!events.contains("not selected"));
        if mode == Mode::Plan {
            assert_eq!(recap["h"].planned, 2);
        } else {
            assert_eq!(recap["h"].ok, 2);
            assert_eq!(recap["h"].failed, 0);
            assert_eq!(recap["h"].skipped, 1);
        }
    }
    Ok(())
}

#[tokio::test]
async fn reached_rescue_and_always_skip_unselected_leaves_with_inherited_censorship() -> Result<()>
{
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"block","noLog":true,"block":[{"name":"failure","tags":["chosen"],"operation":{"kind":"fail"}}],
         "rescue":[{"name":"excluded rescue","operation":{"kind":"fail"}}],
         "always":[{"name":"excluded always","operation":{"kind":"fail"}},
                   {"name":"selected always","tags":["chosen"],"operation":{"kind":"debug","msg":"ok"}}]}
    ]}]});
    let mut backend = Fake::default();
    let options = Options {
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
    assert_eq!(recap["h"].rescued, 1);
    assert_eq!(recap["h"].skipped, 2);
    assert_eq!(recap["h"].failed, 0);
    for (event, name) in backend
        .events
        .iter()
        .skip(1)
        .take(2)
        .zip(["excluded rescue", "excluded always"])
    {
        assert_eq!(event["task"], name);
        assert_eq!(event["result"]["censored"], true);
    }
    assert_eq!(backend.events[3]["task"], "selected always");
    Ok(())
}

#[tokio::test]
async fn always_is_selected_only_when_requested_or_inherited() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"always leaf","tags":["always"],"operation":{"kind":"debug"}},
        {"name":"always block","tags":["always"],"block":[{"name":"inherited leaf","operation":{"kind":"debug"}}]},
        {"name":"other","tags":["other"],"operation":{"kind":"fail"}}
    ]}]});
    for mode in [Mode::Apply, Mode::Plan] {
        let mut backend = Fake::default();
        let options = Options {
            mode,
            tags: vec!["always".into()],
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
        assert!(events.contains("always leaf"));
        assert!(events.contains("inherited leaf"));
        assert_eq!(recap["h"].failed, 0);
        if mode == Mode::Plan {
            assert_eq!(recap["h"].planned, 2);
        } else {
            assert_eq!(recap["h"].ok, 2);
        }
    }
    Ok(())
}

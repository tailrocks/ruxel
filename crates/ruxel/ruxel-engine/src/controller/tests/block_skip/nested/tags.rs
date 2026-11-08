use super::*;

#[tokio::test]
async fn conditional_skips_preserve_leaf_and_block_tag_selection() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"outer","when":false,"block":[
            {"name":"chosen","tags":["chosen"],"register":"chosen","operation":{"kind":"debug"}},
            {"name":"excluded leaf","register":"excluded","operation":{"kind":"fail"}},
            {"name":"excluded block","block":[{"name":"hidden leaf","register":"hidden","operation":{"kind":"fail"}}]},
            {"name":"inherited block","tags":["chosen"],"block":[{"name":"inherited leaf","register":"inherited","operation":{"kind":"debug"}}]},
            {"name":"always leaf","tags":["always"],"register":"always","operation":{"kind":"debug"}}
        ]},
        {"name":"observe","tags":["chosen"],"operation":{"kind":"set_fact","values":{"observed":{"kind":"ref","path":["results"]}}}}
    ]}]});
    let options = Options {
        tags: vec!["chosen".into()],
        ..Options::default()
    };
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let observed = &backend.events.last().context("observer")?["result"]["facts"]["observed"];
    let mut keys: Vec<_> = observed
        .as_object()
        .context("registered results")?
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["always", "chosen", "inherited"]);
    for key in keys {
        assert_eq!(observed[key]["skipped"], true);
        assert_eq!(observed[key]["skip_reason"], "Conditional result was False");
    }
    let names: Vec<_> = backend
        .events
        .iter()
        .filter_map(|event| event["task"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            "chosen",
            "excluded leaf",
            "inherited leaf",
            "always leaf",
            "observe"
        ]
    );
    assert_eq!(recap["h"].skipped, 4);
    assert_eq!(recap["h"].failed, 0);
    assert!(backend.calls.is_empty());
    Ok(())
}

#[tokio::test]
async fn conditional_skips_in_handlers_bypass_task_tag_filters() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"trigger","tags":["chosen"],"changedWhen":true,"notify":["outer","observe"],"operation":{"kind":"debug"}}
    ], "handlers":[skipped_nested_block(false), observe_registered_leaves()]}]});
    let options = Options {
        tags: vec!["chosen".into()],
        ..Options::default()
    };
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].skipped, 2);
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events.len(), 4);
    let observed = &backend.events.last().context("observer")?["result"]["facts"];
    assert_eq!(observed["one"], true);
    assert_eq!(observed["two"], true);
    assert!(backend.calls.is_empty());
    Ok(())
}

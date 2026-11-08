use super::*;

mod tags;

fn skipped_nested_block(private: bool) -> Value {
    json!({"name":"outer", "when":{"kind":"literal","value":false}, "block":[
        {"name":"middle", "noLog":private, "when":{"kind":"ref","path":["vars","missing"]}, "block":[
            {"name":"one", "register":"one", "operation":{"kind":"command","argv":{"kind":"list","items":[{"kind":"ref","path":["vars","missing"]}]}}},
            {"name":"inner", "block":[
                {"name":"two", "register":"two", "loop":{"kind":"ref","path":["vars","missing"]}, "operation":{"kind":"command","argv":["must not run"]}}
            ], "rescue":[{"name":"unentered nested rescue","operation":{"kind":"fail"}}], "always":[{"name":"unentered nested always","operation":{"kind":"fail"}}]}
        ]}
    ], "rescue":[{"name":"unentered rescue","operation":{"kind":"fail"}}], "always":[{"name":"unentered always","operation":{"kind":"fail"}}]})
}

fn observe_registered_leaves() -> Value {
    json!({"name":"observe", "operation":{"kind":"set_fact", "values":{
        "one":{"kind":"ref","path":["results","one","skipped"]},
        "two":{"kind":"ref","path":["results","two","skipped"]},
        "failed":{"kind":"ref","path":["results","two","failed"]},
        "reason":{"kind":"ref","path":["results","one","skip","cause"]}
    }}})
}

#[tokio::test]
async fn skipped_nested_block_registers_only_leaves_in_entered_sections() -> Result<()> {
    for section in ["tasks", "rescue", "always"] {
        let skipped = skipped_nested_block(false);
        let mut tasks = vec![skipped.clone(), observe_registered_leaves()];
        if section != "tasks" {
            let operation = if section == "rescue" { "fail" } else { "debug" };
            let mut wrapper = json!({"name":"wrapper", "block":[{"name":"trigger", "operation":{"kind":operation}}]});
            wrapper[section] = json!([skipped]);
            tasks[0] = wrapper;
        }
        let config =
            json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":tasks}]});
        let mut backend = Fake::default();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recap["h"].skipped, 2, "{section}");
        assert_eq!(recap["h"].failed, 0, "{section}");
        assert_eq!(recap["h"].rescued, u64::from(section == "rescue"));
        let skipped_events: Vec<_> = backend
            .events
            .iter()
            .filter(|event| event["result"]["skipped"] == true)
            .collect();
        assert_eq!(skipped_events.len(), 2, "{section}");
        for (event, name) in skipped_events.iter().zip(["one", "two"]) {
            assert_eq!(event["task"], name);
            assert_eq!(event["result"]["changed"], false);
        }
        assert_eq!(backend.events.len(), if section == "tasks" { 3 } else { 4 });
        let observed = backend.events.last().context("observer event")?;
        assert_eq!(
            observed["result"]["facts"],
            json!({"one":true,"two":true,"failed":false,"reason":"condition_false"})
        );
        assert!(backend.calls.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn skipped_nested_block_inherits_private_registers() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[skipped_nested_block(true), observe_registered_leaves()]}]});
    let mut backend = Fake::default();
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recap["h"].skipped, 2);
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(backend.events.len(), 3);
    for event in &backend.events {
        assert_eq!(event["result"]["censored"], true);
        assert!(event["result"].get("facts").is_none());
        assert!(event["result"].get("skip").is_none());
    }
    assert!(backend.calls.is_empty());
    Ok(())
}

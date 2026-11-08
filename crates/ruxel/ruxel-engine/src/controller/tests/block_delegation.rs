use super::*;

#[tokio::test]
async fn block_delegation_inherits_overrides_and_does_not_leak_to_handlers() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{},"a":{},"b":{}},"groups":{}},"plays":[{
    "targets":["h"],
    "handlers":[{"name":"handler","operation":{"kind":"command","argv":["handler"]}}],
    "tasks":[{"name":"block","delegateTo":{"kind":"ref","path":["vars","destination"]},"delegateAllow":["a"],
        "vars":{"destination":"a"},"noLog":true,
        "block":[
            {"name":"inherited","operation":{"kind":"command","argv":["one"]},"notify":["handler"]},
            {"name":"nested","delegateTo":"b","delegateAllow":["b"],"block":[
                {"name":"override","operation":{"kind":"command","argv":["two"]}},
                {"name":"failure","operation":{"kind":"command","argv":["three"]}}
            ],"rescue":[{"name":"rescue","operation":{"kind":"command","argv":["four"]}}],
              "always":[{"name":"always","operation":{"kind":"command","argv":["five"]}}]},
            {"name":"restored","operation":{"kind":"command","argv":["six"]}}
        ]}]}]});
    let mut backend = Fake {
        results: [
            json!({"changed":true}),
            json!({}),
            json!({"failed":true}),
            json!({}),
            json!({}),
            json!({}),
            json!({}),
        ]
        .into(),
        ..Fake::default()
    };
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let hosts: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.host.as_str())
        .collect();
    assert_eq!(hosts, ["a", "b", "b", "b", "b", "a", "h"]);
    assert!(backend.calls[..6].iter().all(|call| call.task.no_log));
    Ok(())
}

#[tokio::test]
async fn inherited_expression_uses_each_iteration_and_leaf_override() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{},"a":{},"b":{}},"groups":{}},"plays":[{
    "targets":["h"],"tasks":[{"name":"block",
        "delegateTo":{"kind":"ref","path":["item"]},"delegateAllow":["a","b"],"noLog":true,
        "block":[
            {"name":"loop","loop":["a","b"],"operation":{"kind":"command","argv":["loop"]}},
            {"name":"override","delegateTo":"a","delegateAllow":["a"],"operation":{"kind":"command","argv":["override"]}}
        ]}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let hosts: Vec<_> = backend
        .calls
        .iter()
        .map(|call| call.host.as_str())
        .collect();
    assert_eq!(hosts, ["a", "b", "a"]);
    assert!(backend.calls.iter().all(|call| call.task.no_log));
    Ok(())
}

#[tokio::test]
async fn dynamic_delegation_requires_task_allowlist_and_localhost_opt_in() -> Result<()> {
    for (destination, allow, local, succeeds) in [
        ("a", vec!["a"], false, true),
        ("b", vec!["a"], false, false),
        ("localhost", vec!["localhost"], false, false),
        ("localhost", vec!["localhost"], true, true),
        ("b", vec![], false, false),
    ] {
        let config = json!({"inventory":{"hosts":{"h":{},"a":{},"b":{}},"groups":{}},
            "plays":[{"targets":["h"],"tasks":[{"name":"delegate",
            "delegateTo":{"kind":"ref","path":["facts","target"]},
            "delegateAllow":allow,"delegateLocalhost":local,
            "operation":{"kind":"command","argv":["fixture"]}}]}]});
        let facts = BTreeMap::from([("h".to_owned(), json!({"target":destination}))]);
        let mut backend = Fake::default();
        let result = run(
            &config,
            &facts,
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(result.is_ok(), succeeds, "target {destination}");
        assert_eq!(
            backend.calls.len(),
            usize::from(succeeds),
            "target {destination}"
        );
        if succeeds {
            assert_eq!(backend.calls[0].host, destination);
        }
    }
    Ok(())
}

#[tokio::test]
async fn inherited_delegation_rejects_child_authorization_without_new_target() -> Result<()> {
    for override_fields in [
        json!({"delegateAllow":["a"]}),
        json!({"delegateLocalhost":false}),
    ] {
        let mut child = json!({"name":"child", "operation":{"kind":"command","argv":["fixture"]}});
        child.as_object_mut().context("child object")?.extend(
            override_fields
                .as_object()
                .context("override object")?
                .clone(),
        );
        let config = json!({"inventory":{"hosts":{"h":{},"a":{}},"groups":{}},
            "plays":[{"targets":["h"],"tasks":[{"name":"parent",
            "delegateTo":"localhost", "delegateAllow":["localhost","a"],
            "delegateLocalhost":true, "block":[child]}]}]});
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
        .context("delegation override must be rejected")?;
        assert!(
            error
                .to_string()
                .contains("authorization requires delegateTo")
        );
        assert!(backend.calls.is_empty());
    }
    Ok(())
}

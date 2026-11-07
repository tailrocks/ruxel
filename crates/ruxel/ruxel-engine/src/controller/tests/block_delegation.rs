use super::*;

#[tokio::test]
async fn block_delegation_inherits_overrides_and_does_not_leak_to_handlers() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{},"a":{},"b":{}},"groups":{}},"plays":[{
    "targets":["h"],
    "handlers":[{"name":"handler","operation":{"kind":"command","cmd":"handler"}}],
    "tasks":[{"name":"block","delegateTo":{"kind":"ref","path":["vars","destination"]},
        "vars":{"destination":"a"},"noLog":true,
        "block":[
            {"name":"inherited","operation":{"kind":"command","cmd":"one"},"notify":["handler"]},
            {"name":"nested","delegateTo":"b","block":[
                {"name":"override","operation":{"kind":"command","cmd":"two"}},
                {"name":"failure","operation":{"kind":"command","cmd":"three"}}
            ],"rescue":[{"name":"rescue","operation":{"kind":"command","cmd":"four"}}],
              "always":[{"name":"always","operation":{"kind":"command","cmd":"five"}}]},
            {"name":"restored","operation":{"kind":"command","cmd":"six"}}
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
        "delegateTo":{"kind":"ref","path":["item"]},"noLog":true,
        "block":[
            {"name":"loop","loop":["a","b"],"operation":{"kind":"command","cmd":"loop"}},
            {"name":"override","delegateTo":"a","operation":{"kind":"command","cmd":"override"}}
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

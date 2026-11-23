use super::*;

#[tokio::test]
async fn loop_summary_fields_are_available_in_registered_results() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"empty","register":"empty","loop":{"kind":"literal","value":[]},"operation":{"kind":"command","argv":["unused"]}},
        {"name":"complete","register":"complete","loop":{"kind":"literal","value":[1]},"operation":{"kind":"command","argv":["ok"]}},
        {"name":"skipped","register":"skipped","loop":{"kind":"literal","value":[1]},"when":false,"operation":{"kind":"command","argv":["unused"]}},
        {"name":"failure","register":"failure","loop":{"kind":"literal","value":[1]},"ignoreErrors":true,"operation":{"kind":"fail"}},
        {"name":"observe","operation":{"kind":"set_fact","values":{"empty":{"kind":"ref","path":["results","empty","skip","cause"]},"complete":{"kind":"ref","path":["results","complete","msg"]},"skipped":{"kind":"ref","path":["results","skipped","msg"]},"failure":{"kind":"ref","path":["results","failure","msg"]}}}}
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
    assert_eq!(recap["h"].failed, 0);
    assert_eq!(recap["h"].ignored, 1);
    assert_eq!(
        backend.events[0]["result"],
        json!({"changed":false,"failed":false,"skipped":true,"results":[],"skip":{"cause":"empty_loop"}})
    );
    assert_eq!(
        backend.events[4]["result"]["facts"],
        json!({"empty":"empty_loop","complete":"All items completed","skipped":"All items skipped","failure":"One or more items failed"})
    );
    Ok(())
}

#[tokio::test]
async fn empty_loop_preserves_source_sensitivity_in_events_and_registered_result() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"public loop","loop":{"kind":"literal","value":[]},"operation":{"kind":"debug","msg":"unused"}},
        {"name":"private fact","noLog":true,"operation":{"kind":"set_fact","values":{"privateList":{"kind":"literal","value":[]}}}},
        {"name":"private loop","register":"privateLoop","loop":{"kind":"ref","path":["vars","privateList"]},"operation":{"kind":"debug","msg":"unused"}},
        {"name":"private result","operation":{"kind":"set_fact","values":{"reason":{"kind":"ref","path":["results","privateLoop","skip","cause"]}}}}
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
    assert_eq!(recap["h"].skipped, 2);
    assert_eq!(backend.events[0]["result"]["skip"]["cause"], "empty_loop");
    assert_eq!(backend.events[2]["result"]["censored"], true);
    assert_eq!(backend.events[3]["result"]["censored"], true);
    Ok(())
}

#[tokio::test]
async fn empty_loop_in_private_task_or_block_keeps_registered_result_private() -> Result<()> {
    for inherited in [false, true] {
        let empty = json!({"name":"private loop","noLog":!inherited,"register":"privateLoop","loop":{"kind":"literal","value":[]},"operation":{"kind":"debug","msg":"unused"}});
        let private = if inherited {
            json!({"name":"private block","noLog":true,"block":[empty]})
        } else {
            empty
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[private,
            {"name":"observe","operation":{"kind":"set_fact","values":{"reason":{"kind":"ref","path":["results","privateLoop","skip","cause"]}}}}
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
        assert_eq!(recap["h"].skipped, 1);
        assert_eq!(backend.events[0]["result"]["censored"], true);
        assert_eq!(backend.events[1]["result"]["censored"], true);
    }
    Ok(())
}

#[tokio::test]
async fn sensitive_parent_condition_keeps_empty_loop_result_private() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"private condition","noLog":true,"operation":{"kind":"set_fact","values":{"gate":true}}},
        {"name":"guarded block","when":{"kind":"ref","path":["vars","gate"]},"block":[{"name":"empty","register":"empty","loop":{"kind":"literal","value":[]},"operation":{"kind":"debug"}}]},
        {"name":"observe","operation":{"kind":"set_fact","values":{"reason":{"kind":"ref","path":["results","empty","skip","cause"]}}}}
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
    assert_eq!(recap["h"].skipped, 1);
    assert_eq!(backend.events[1]["result"]["censored"], true);
    assert_eq!(backend.events[2]["result"]["censored"], true);
    Ok(())
}

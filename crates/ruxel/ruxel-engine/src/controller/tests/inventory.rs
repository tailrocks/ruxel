use super::*;

#[tokio::test]
async fn child_group_variables_override_parents_regardless_of_names() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{"a-child":{"hosts":["h"],"vars":{"value":{"kind":"literal","value":"child"}}},"z-parent":{"children":["a-child"],"vars":{"value":{"kind":"literal","value":"parent"}}}}},"plays":[{"targets":["all"],"tasks":[{"name":"test","operation":{"kind":"command","cmd":{"kind":"ref","path":["vars","value"]}}}]}]});
    let mut backend = Fake::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    let params: Value = serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
    assert_eq!(params["cmd"], "child");
    Ok(())
}

#[tokio::test]
async fn plan_counts_selected_operations_without_evaluating_them() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["all"],"tasks":[{"name":"chosen","tags":["chosen"],"operation":{"kind":"command","cmd":{"kind":"ref","path":["results","notAvailable"]}}},{"name":"excluded","operation":{"kind":"command","cmd":"x"}},{"name":"block","tags":["chosen"],"block":[{"name":"nested","operation":{"kind":"command","cmd":"y"}}]},{"name":"always","tags":["always"],"operation":{"kind":"command","cmd":"z"}}],"handlers":[{"name":"possible","operation":{"kind":"command","cmd":"handler"}}]}]});
    let mut backend = Fake::default();
    let options = Options {
        mode: Mode::Plan,
        tags: vec!["chosen".into()],
        ..Options::default()
    };
    let recaps = run(
        &config,
        &BTreeMap::new(),
        &options,
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(recaps["h"].planned, 2);
    assert_eq!(recaps["h"].changed, 0);
    assert_eq!(backend.calls.len(), 0);
    assert_eq!(backend.events.len(), 3);
    assert!(
        !backend
            .events
            .iter()
            .any(|event| event["task"]["name"] == "excluded")
    );
    Ok(())
}

#[tokio::test]
async fn task_check_mode_override_is_forwarded_for_check_and_diff() -> Result<()> {
    for mode in [Mode::Check, Mode::Diff] {
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["all"],"tasks":[{"name":"default","operation":{"kind":"command","cmd":"true"}},{"name":"forced","checkMode":true,"operation":{"kind":"command","cmd":"true"}},{"name":"override","checkMode":false,"operation":{"kind":"command","cmd":"true"}}]}]});
        let options = Options {
            mode,
            ..Options::default()
        };
        let mut backend = Fake::default();
        run(
            &config,
            &BTreeMap::new(),
            &options,
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert!(backend.calls.iter().all(|call| call.task.force_check_mode));
        assert!(!backend.calls[0].task.check_mode_override);
        assert!(!backend.calls[1].task.check_mode_override);
        assert!(backend.calls[2].task.check_mode_override);
    }
    Ok(())
}

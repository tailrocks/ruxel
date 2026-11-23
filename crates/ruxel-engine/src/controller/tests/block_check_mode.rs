use super::*;

#[tokio::test]
async fn block_check_mode_inherits_and_child_overrides_reach_agent_requests() -> Result<()> {
    for mode in [Mode::Apply, Mode::Check, Mode::Diff] {
        for block_check in [false, true] {
            let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
                {"name":"block","checkMode":block_check,"block":[
                    {"name":"inherited","operation":{"kind":"command","argv":["unused"]}},
                    {"name":"nested","block":[{"name":"nested inherited","operation":{"kind":"command","argv":["unused"]}}]},
                    {"name":"override block","checkMode":!block_check,"block":[{"name":"nested override","operation":{"kind":"command","argv":["unused"]}}]},
                    {"name":"override task","checkMode":!block_check,"operation":{"kind":"command","argv":["unused"]}},
                    {"name":"failure","operation":{"kind":"fail"}}
                ],"rescue":[{"name":"rescue","operation":{"kind":"command","argv":["unused"]}}],
                  "always":[{"name":"always","operation":{"kind":"command","argv":["unused"]}}]},
                {"name":"outside","operation":{"kind":"command","argv":["unused"]}}
            ]}]});
            let mut backend = Fake::default();
            let options = Options {
                mode,
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
            assert_eq!(recaps["h"].failed, 0);
            assert_eq!(recaps["h"].rescued, 1);
            assert_eq!(backend.calls.len(), 7);
            for (call, setting) in backend.calls.iter().zip([
                Some(block_check),
                Some(block_check),
                Some(!block_check),
                Some(!block_check),
                Some(block_check),
                Some(block_check),
                None,
            ]) {
                assert_eq!(
                    call.task.check_mode_override,
                    setting == Some(false),
                    "{}",
                    call.task.name
                );
                assert_eq!(
                    call.task.force_check_mode,
                    matches!(mode, Mode::Check | Mode::Diff) || setting == Some(true),
                    "{}",
                    call.task.name
                );
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn block_check_mode_controls_local_pause_and_child_opt_out() -> Result<()> {
    for mode in [Mode::Apply, Mode::Check, Mode::Diff] {
        for block_check in [false, true] {
            let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
                {"name":"block","checkMode":block_check,"block":[
                    {"name":"inherited","operation":{"kind":"pause","seconds":0}},
                    {"name":"override","checkMode":!block_check,"operation":{"kind":"pause","seconds":0}}
                ]}
            ]}]});
            let options = Options {
                mode,
                ..Options::default()
            };
            let mut backend = Fake::default();
            let recaps = run(
                &config,
                &BTreeMap::new(),
                &options,
                &mut backend,
                &mut BTreeMap::new(),
            )
            .await?;
            assert_eq!(recaps["h"].failed, 0);
            assert_eq!(recaps["h"].skipped, 1);
            assert_eq!(
                backend
                    .events
                    .iter()
                    .filter(|event| event.get("pause").is_some())
                    .count(),
                0
            );
            let results: Vec<_> = backend
                .events
                .iter()
                .filter(|event| event.get("result").is_some())
                .collect();
            assert_eq!(results.len(), 2);
            assert_eq!(
                results[0]["result"]["skipped"].as_bool().unwrap_or(false),
                block_check
            );
            assert_eq!(
                results[1]["result"]["skipped"].as_bool().unwrap_or(false),
                !block_check
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn interactive_pause_without_terminal_stops_following_tasks() {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return;
    }
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"approval","operation":{"kind":"pause"}},
        {"name":"must not run","operation":{"kind":"command","argv":["unused"]}}
    ]}]});
    let mut backend = Fake::default();
    let result = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await;
    assert!(result.is_err());
    assert!(backend.calls.is_empty());
}

#[tokio::test]
async fn zero_duration_pauses_do_not_prompt() -> Result<()> {
    for operation in [
        json!({"kind":"pause","seconds":0}),
        json!({"kind":"pause","minutes":0}),
    ] {
        let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[{"name":"pause","operation":operation}]}]});
        let mut backend = Fake::default();
        let recaps = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recaps["h"].failed, 0);
        assert!(
            !backend
                .events
                .iter()
                .any(|event| event.get("pause").is_some())
        );
    }
    Ok(())
}

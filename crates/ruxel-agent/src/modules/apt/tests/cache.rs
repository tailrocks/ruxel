use super::*;

#[test]
fn failed_update_invalidates_cached_packages_and_reports_possible_change() {
    let mut state = SystemState::with_test_packages();
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let result = run_with(
        &json!({"update_cache": true}),
        &ctx,
        &mut state,
        &mut |_, _| Ok((String::new(), 1, "failure".into())),
    )
    .unwrap();
    assert_eq!(result["failed"], true);
    assert_eq!(result["changed"], true);
    assert!(state.test_packages_invalidated());
}

#[test]
fn cache_refresh_reports_change_only_when_applied() -> Result<(), String> {
    for check in [false, true] {
        for with_maintenance in [false, true] {
            let mut calls = Vec::new();
            let mut state = SystemState::with_test_packages();
            let result = run_with(
                &json!({"update_cache": true, "autoremove": with_maintenance}),
                &context(check),
                &mut state,
                &mut |args, _| {
                    calls.push(args.iter().map(ToString::to_string).collect::<Vec<_>>());
                    Ok((summary(false), 0, String::new()))
                },
            )?;
            assert_eq!(result["changed"], !check);
            assert_eq!(result["cache_updated"], !check);
            assert_eq!(result["failed"], false);
            assert_eq!(state.test_packages_invalidated(), !check);
            let mut expected = Vec::new();
            if !check {
                expected.push(vec!["update"]);
            }
            if with_maintenance {
                expected.push(if check {
                    vec!["-s", "autoremove"]
                } else {
                    vec!["autoremove"]
                });
            }
            assert_eq!(calls, expected);
        }
    }
    Ok(())
}

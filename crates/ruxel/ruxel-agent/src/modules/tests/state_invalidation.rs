use crate::system_state::SystemState;
use serde_json::json;

fn context(check_mode: bool) -> super::ExecContext {
    super::ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

#[test]
fn commands_that_run_invalidate_package_and_unit_snapshots() {
    for (module, params, free_form) in [
        ("command", json!({"argv": ["true"]}), ""),
        ("shell", json!({}), "true"),
        ("command", json!({"argv": ["false"]}), ""),
    ] {
        let mut state = SystemState::with_test_units();
        let outcome = super::execute(
            module,
            &params,
            free_form,
            None,
            &context(false),
            &mut state,
        );
        assert_ne!(outcome.status, "skipped", "{module} {params}");
        assert!(state.test_packages_invalidated(), "{module} {params}");
        assert!(state.test_units_invalidated(), "{module} {params}");
    }
}

#[test]
fn check_mode_commands_keep_the_snapshots() {
    for (module, params, free_form) in [
        ("command", json!({"argv": ["true"]}), ""),
        ("shell", json!({}), "true"),
    ] {
        let mut state = SystemState::with_test_units();
        let _skipped = super::execute(module, &params, free_form, None, &context(true), &mut state);
        assert!(!state.test_packages_invalidated(), "{module}");
        assert!(!state.test_units_invalidated(), "{module}");
    }
}

#[test]
fn modules_that_cannot_change_packages_or_units_keep_the_snapshots() {
    let mut state = SystemState::with_test_units();
    let _result = super::execute(
        "stat",
        &json!({"path": "/"}),
        "",
        None,
        &context(false),
        &mut state,
    );
    assert!(!state.test_packages_invalidated());
    assert!(!state.test_units_invalidated());
}

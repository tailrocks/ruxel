use super::*;

#[test]
fn command_accepts_only_nonempty_argv() -> Result<()> {
    for value in [
        json!({}),
        json!({"cmd":"echo hi"}),
        json!({"argv":"echo hi"}),
        json!({"argv":[]}),
        json!({"argv":["echo"],"cmd":"echo"}),
    ] {
        assert!(
            validate_operation("command", value.as_object().context("object")?).is_err(),
            "{value}"
        );
    }
    assert!(
        validate_operation(
            "command",
            json!({"argv":["echo", "a b", ""]})
                .as_object()
                .context("object")?
        )
        .is_ok()
    );
    Ok(())
}

/// The service operation's state is a closed surface: `reloaded` asks the
/// unit to reload its configuration, beside the restart, and every other
/// spelling is refused before the agent sees it.
#[test]
fn a_service_state_is_one_of_the_closed_surface() -> Result<()> {
    for state in ["started", "stopped", "restarted", "reloaded"] {
        validate_operation(
            "systemd",
            json!({"name":"fixture.service","state":state})
                .as_object()
                .context("object")?,
        )?;
    }
    for state in ["reload", "reloaded!", "running", ""] {
        assert!(
            validate_operation(
                "systemd",
                json!({"name":"fixture.service","state":state})
                    .as_object()
                    .context("object")?,
            )
            .is_err(),
            "{state}"
        );
    }
    Ok(())
}

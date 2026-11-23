use super::*;

#[test]
fn debconf_type_is_one_of_the_six_debconf_types() -> Result<()> {
    for kind in [
        "string",
        "boolean",
        "select",
        "multiselect",
        "password",
        "note",
    ] {
        let value = json!({"package": "nginx", "question": "nginx/nginx-user", "type": kind, "value": "www-data"});
        assert!(
            validate_operation("debconf", value.as_object().context("object")?).is_ok(),
            "{kind}"
        );
    }
    // The removed `text` type stays out of the engine surface: the
    // workflow side (`Debian.pkl`) refuses it, and a `text` question the
    // engine admitted would never converge — `debconf-get-selections`
    // skips text questions while `debconf-set-selections` accepts them,
    // so every apply would rewrite the answer indefinitely.
    let value = json!({"package": "nginx", "question": "nginx/nginx-user", "type": "text", "value": "www-data"});
    assert!(
        validate_operation("debconf", value.as_object().context("object")?).is_err(),
        "the removed text type must not pass the engine surface"
    );
    Ok(())
}

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

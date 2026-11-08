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

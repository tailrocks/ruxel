use super::*;

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    }
}

#[test]
fn a_different_answer_is_written_and_reported_as_a_change() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut || Ok(String::new()),
        &mut |selection| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(result["failed"], false);
    assert_eq!(written, ["nginx nginx/nginx-user string www-data\n"]);
    Ok(())
}

#[test]
fn the_same_answer_is_not_written_again() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut || Ok("nginx nginx/nginx-user string www-data\n".to_owned()),
        &mut |selection| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], false);
    assert!(written.is_empty(), "{written:?}");
    Ok(())
}

#[test]
fn check_mode_writes_nothing_and_reports_the_pending_change() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(true),
        &mut || Ok(String::new()),
        &mut |selection| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert!(written.is_empty(), "{written:?}");
    Ok(())
}

#[test]
fn the_answer_reaches_the_host_on_the_standard_input_only() -> Result<(), String> {
    let mut written = Vec::new();
    let secret = "s3cr3t-value";
    let result = run_with(
        &json!({"package": "postfix", "question": "postfix/mailname", "type": "password", "value": secret}),
        &context(false),
        &mut || Ok(String::new()),
        &mut |selection| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(
        written,
        ["postfix postfix/mailname password s3cr3t-value\n"]
    );
    // The answer is written as it is: no quoting, no escaping, no argument.
    assert!(
        written[0].ends_with(&format!(" {secret}\n")),
        "{:?}",
        written[0]
    );
    Ok(())
}

#[test]
fn an_answer_with_a_line_break_is_refused() {
    let error = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "a\nb"}),
        &context(false),
        &mut || Ok(String::new()),
        &mut |_| Ok(()),
    )
    .expect_err("refused");
    assert!(error.contains("one line"), "{error}");
}

#[test]
fn a_missing_field_is_refused() {
    for params in [
        json!({"question": "q", "type": "string", "value": "v"}),
        json!({"package": "p", "type": "string", "value": "v"}),
        json!({"package": "p", "question": "q", "value": "v"}),
        json!({"package": "p", "question": "q", "type": "string"}),
    ] {
        let error = run_with(
            &params,
            &context(false),
            &mut || Ok(String::new()),
            &mut |_| Ok(()),
        )
        .expect_err("refused");
        assert!(error.starts_with("debconf: "), "{error}");
    }
}

#[test]
fn the_answer_of_one_question_is_read_from_the_selections() {
    let selections =
        "nginx nginx/nginx-user string www-data\npostfix postfix/mailname string mail\n";
    assert_eq!(
        answer_of(selections, "postfix", "postfix/mailname").as_deref(),
        Some("mail")
    );
    assert_eq!(
        answer_of(selections, "nginx", "nginx/nginx-user").as_deref(),
        Some("www-data")
    );
    assert_eq!(answer_of(selections, "nginx", "nginx/other"), None);
}

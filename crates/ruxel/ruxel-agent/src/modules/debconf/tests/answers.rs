use super::*;

#[test]
fn a_different_answer_is_written_and_reported_as_a_change() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok(String::new()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
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
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
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
        &mut |_| Ok(String::new()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
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
        &mut |_| Ok(String::new()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
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
        &mut |_| Ok(String::new()),
        &mut |_, _, _| Ok(None),
        &mut |_, _| Ok(()),
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
            &mut |_| Ok(String::new()),
            &mut |_, _, _| Ok(None),
            &mut |_, _| Ok(()),
        )
        .expect_err("refused");
        assert!(error.starts_with("debconf: "), "{error}");
    }
}

#[test]
fn the_type_of_the_question_is_compared_with_the_type_of_the_answer() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user select www-data\n"]);
    Ok(())
}

#[test]
fn the_same_type_and_the_same_value_are_no_change() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], false);
    assert!(written.is_empty(), "{written:?}");
    Ok(())
}

#[test]
fn a_boolean_answer_is_true_or_false_and_nothing_else() {
    for value in ["yes", "banana", "True", ""] {
        let error = run_with(
            &json!({"package": "nginx", "question": "nginx/enable", "type": "boolean", "value": value}),
            &context(false),
            &mut |_| Ok(String::new()),
            &mut |_, _, _| Ok(None),
            &mut |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(
            error.contains("\"true\"") && error.contains("\"false\""),
            "{value:?}: {error}"
        );
    }
    // The canonical spellings pass: debconf frontends read them as the
    // boolean they name.
    for value in ["true", "false"] {
        run_with(
            &json!({"package": "nginx", "question": "nginx/enable", "type": "boolean", "value": value}),
            &context(false),
            &mut |_| Ok(String::new()),
            &mut |_, _, _| Ok(None),
            &mut |_, _| Ok(()),
        )
        .unwrap_or_else(|error| panic!("{value:?}: {error}"));
    }
}

/// The protocol reply of `debconf-communicate` decides, not its exit
/// status: the tool exits with the code of the last reply, so a question
/// the database does not know answers `10 ...` and exits 10. A first-time
/// preseed must see that as "no answer" and proceed to
/// `debconf-set-selections`, not fail.
#[test]
fn a_missing_question_reply_is_no_answer_even_when_the_tool_exits_with_it() {
    assert_eq!(
        communicate_reply("10 nginx/nginx-user doesn't exist\n", "", false),
        Ok(None)
    );
}

#[test]
fn a_zero_reply_answers_the_value_whatever_the_exit_status() {
    assert_eq!(
        communicate_reply("0 www-data\n", "", false),
        Ok(Some("www-data".to_owned()))
    );
}

#[test]
fn no_protocol_reply_and_a_failure_stay_a_failure() {
    assert!(communicate_reply("", "boom\n", false).is_err());
    assert!(communicate_reply("surprise\n", "", true).is_err());
}

/// Marking the question seen is part of `debconf-set-selections`' effect:
/// an administrator resetting the `seen` flag must bring the task back,
/// or the next package configuration prompts or picks a frontend default
/// even though the answer matches.
#[test]
fn an_unseen_answer_is_written_again_to_mark_it_seen() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| {
            Ok(Some(Reply {
                value: "www-data".to_owned(),
                seen: false,
            }))
        },
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user string www-data\n"]);
    Ok(())
}

/// The fallback comparison reconciles the flag along with the value.
#[test]
fn an_unseen_answer_the_fallback_reads_is_written_again() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| {
            Ok(Some(Reply {
                value: "www-data".to_owned(),
                seen: false,
            }))
        },
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user select www-data\n"]);
    Ok(())
}

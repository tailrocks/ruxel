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

/// Every non-Apply mode resolves secrets with the synthetic resolver, so
/// a boolean `secretAnswer(...)` arrives as `dry-secret-…` here: the
/// boolean check is about what the setter would write, and check and diff
/// write nothing, so it runs in apply mode only.
#[test]
fn a_boolean_secret_passes_check_mode_without_error() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/worker", "type": "boolean", "value": "dry-secret-0123456789abcdef"}),
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
fn the_same_answer_is_not_written_again() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| Ok(Some(Reply { seen: true })),
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
        &mut |_, _, _| Ok(Some(Reply { seen: true })),
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
        &mut |_, _, _| Ok(Some(Reply { seen: false })),
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user string www-data\n"]);
    Ok(())
}

/// The fallback writes an unseen answer again — as it writes every
/// answer, the type being unverifiable there.
#[test]
fn an_unseen_answer_the_fallback_reads_is_written_again() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| Ok(Some(Reply { seen: false })),
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user select www-data\n"]);
    Ok(())
}

/// The seen flag the value reader answers is part of the comparison: a
/// reader that fails is a failure, not a silent "seen" — a question an
/// administrator reset must never be papered over by a broken reader.
#[test]
fn a_failing_seen_query_is_reported_and_not_counted_as_seen() {
    let error = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
        &mut |_, _, _| Err("debconf-communicate failed: boom".into()),
        &mut |_, _| panic!("nothing must be written"),
    )
    .expect_err("refused");
    assert!(error.contains("boom"), "{error}");
}

/// `GETSEEN` is not a command the protocol defines: `debconf-communicate`
/// answers `20 Unsupported command "getseen"` — and the reader sent that
/// very command. The old parser turned the nonzero reply into `Ok(None)`
/// and the seen query read it as `seen=true`, so a question whose `seen`
/// flag an administrator reset claimed convergence. An unexpected reply
/// must surface as an error, never as no answer read for "seen": the
/// supported query is `FGET question seen`, whose answers are `0 true`
/// and `0 false`.
#[test]
fn an_unsupported_command_reply_is_not_read_as_the_seen_flag() {
    let reply = communicate_reply("20 Unsupported command \"getseen\"\n", "", true);
    assert!(
        reply.is_err(),
        "an unsupported command reply must surface as an error, never as no answer read for seen: {reply:?}"
    );
}

/// The whole seen query with the `GETSEEN` reply fails: `communicate_reply`
/// surfaces the error and `communicate_seen` turns no answer — which the
/// old reader swallowed into `seen=true` — into an error as well.
#[test]
fn an_unsupported_command_reply_fails_the_seen_query() {
    let seen = communicate_reply("20 Unsupported command \"getseen\"\n", "", true)
        .and_then(|reply| communicate_seen(reply.as_deref()));
    assert!(seen.is_err(), "{seen:?}");
}

/// The only answers `FGET question seen` gives are `0 true` and
/// `0 false`; nothing else becomes a seen flag.
#[test]
fn the_fget_reply_answers_the_seen_flag_and_nothing_else() {
    assert_eq!(communicate_seen(Some("true")), Ok(true));
    assert_eq!(communicate_seen(Some("false")), Ok(false));
    assert!(communicate_seen(Some("banana")).is_err());
    assert!(communicate_seen(None).is_err());
}

/// A question the value reader cannot find is written again: the database
/// reader printed a row the question no longer backs — it was deleted
/// between the two readers — and a stale row must not count as seen, or
/// the apply reports a convergence the host does not have. The test this
/// replaces asserted `changed = false` and a silent write, endorsing
/// exactly that false convergence; it is changed deliberately.
#[test]
fn a_question_the_value_reader_cannot_find_is_written_again() -> Result<(), String> {
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
    assert_eq!(result["changed"], true);
    assert_eq!(
        written,
        vec!["nginx nginx/nginx-user string www-data\n".to_owned()]
    );
    Ok(())
}

/// A resolved selector carries the grammar the Engine surface admits as
/// literals: the `package question type value` line shifts its fields
/// when a selector embeds whitespace, so the setter could target a
/// different question while the command exits successfully. The runtime
/// refuses the resolved value, expressions included.
#[test]
fn a_resolved_selector_with_whitespace_is_refused() -> Result<(), String> {
    for (package, question) in [("a b", "nginx/user"), ("nginx", "a b"), ("nginx", "a\tb")] {
        let mut written = Vec::new();
        let Err(error) = run_with(
            &json!({"package": package, "question": question, "type": "string", "value": "x"}),
            &context(false),
            &mut |_| Ok(String::new()),
            &mut |_, _, _| Ok(None),
            &mut |selection, _| {
                written.push(selection.to_owned());
                Ok(())
            },
        ) else {
            return Err(format!(
                "{package:?}/{question:?}: the whitespace selector was admitted"
            ));
        };
        assert!(error.contains("debconf:"), "{error}");
        assert!(written.is_empty(), "{package:?}/{question:?}: {written:?}");
    }
    Ok(())
}

/// A resolved package must be an exact package name: a glob names no
/// owner and the answer would never reach the question it names.
#[test]
fn a_resolved_package_that_is_not_a_name_is_refused() -> Result<(), String> {
    let Err(error) = run_with(
        &json!({"package": "nginx*", "question": "nginx/user", "type": "string", "value": "x"}),
        &context(false),
        &mut |_| Ok(String::new()),
        &mut |_, _, _| Ok(None),
        &mut |_, _| Ok(()),
    ) else {
        return Err("the glob package was admitted".into());
    };
    assert!(error.contains("exact package name"), "{error}");
    Ok(())
}

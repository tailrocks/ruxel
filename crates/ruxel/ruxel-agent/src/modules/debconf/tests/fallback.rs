//! The reader fallback: `debconf-get-selections` is only a Suggests of
//! `debconf`, so a host with base `debconf` alone must still converge.
//! There the question's value is read with `debconf-communicate`, and the
//! comparison sees the value only: the type belongs to the template.

use super::*;

#[test]
fn a_missing_reader_falls_back_to_the_value_of_the_question() -> Result<(), String> {
    let mut asked = Vec::new();
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |package, question, _| {
            asked.push(format!("{package} {question}"));
            Ok(Some(Reply {
                value: "www-data".to_owned(),
                seen: true,
            }))
        },
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    // The value is the one the host holds, so nothing is written: the
    // fallback compares the value alone and leaves the type to the
    // template.
    assert_eq!(result["changed"], false);
    assert!(written.is_empty(), "{written:?}");
    assert_eq!(asked, ["nginx nginx/nginx-user"]);
    Ok(())
}

#[test]
fn the_fallback_writes_a_value_that_differs() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| {
            Ok(Some(Reply {
                value: "nginx".to_owned(),
                seen: true,
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

#[test]
fn the_fallback_sets_a_question_the_host_does_not_know() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| Ok(None),
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user string www-data\n"]);
    Ok(())
}

/// A reader that runs and fails is a failure, not a missing tool: the
/// fallback must not paper over a broken `debconf-get-selections`.
#[test]
fn a_failing_reader_is_reported_and_not_fallen_back_from() {
    let mut asked = false;
    let error = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::Failed("debconf-get-selections failed: boom".into())),
        &mut |_, _, _| {
            asked = true;
            Ok(None)
        },
        &mut |_, _| Ok(()),
    )
    .expect_err("refused");
    assert!(error.contains("boom"), "{error}");
    assert!(!asked, "the fallback must not run");
}

/// A fallback that cannot run fails with an actionable error, before any
/// answer is written.
#[test]
fn a_failing_fallback_is_reported() {
    let error = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| Err("debconf: debconf-communicate is not installed; install the debconf package".into()),
        &mut |_, _| panic!("nothing must be written"),
    )
    .expect_err("refused");
    assert!(error.contains("install the debconf package"), "{error}");
}

/// A tool that is not installed is a missing tool, not a failure: the
/// fallback keys off exactly this. A tool that runs and fails is a
/// failure.
#[test]
fn a_tool_that_is_not_installed_reads_as_a_missing_tool() {
    assert!(matches!(
        current_selections_named("ruxel-no-such-debconf-reader", &context(false)),
        Err(ReadError::MissingTool)
    ));
    assert!(matches!(
        current_selections_named("false", &context(false)),
        Err(ReadError::Failed(_))
    ));
}

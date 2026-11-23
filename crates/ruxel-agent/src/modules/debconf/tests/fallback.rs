//! The reader fallback: `debconf-get-selections` is only a Suggests of
//! `debconf`, so a host with base `debconf` alone must still converge.
//! There the question's value is read with `debconf-communicate`, which
//! answers the value and the seen flag but never the type of the
//! template: the comparison is conservative and writes the selection
//! again, so a wrong-typed answer is corrected instead of kept.

use super::*;

#[test]
fn the_fallback_rewrites_the_selection_because_the_type_cannot_be_verified() -> Result<(), String> {
    let mut asked = Vec::new();
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |package, question, _| {
            asked.push(format!("{package} {question}"));
            Ok(Some(Reply { seen: true }))
        },
        &mut |selection, _| {
            written.push(selection.to_owned());
            Ok(())
        },
    )?;
    // The value reader answers the value and the seen flag but never the
    // type of the template, so a wrong-typed answer would compare equal
    // and never be caught: the selection is written again —
    // conservatively — and the rewrite corrects a type that drifted.
    assert_eq!(result["changed"], true);
    assert_eq!(written, ["nginx nginx/nginx-user select www-data\n"]);
    assert_eq!(asked, ["nginx nginx/nginx-user"]);
    Ok(())
}

/// Whatever the value reader answers, the selection is written: its
/// answer carries no type, so it can never make the rewrite unnecessary.
#[test]
fn the_fallback_writes_whatever_the_value_reader_answers() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "select", "value": "www-data"}),
        &context(false),
        &mut |_| Err(ReadError::MissingTool),
        &mut |_, _, _| Ok(Some(Reply { seen: true })),
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

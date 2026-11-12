//! What the module reads out of the selections of the host.
//!
//! `debconf-get-selections` prints the database as TAB separated fields,
//! `owner<TAB>question<TAB>type<TAB>value`; the value is the rest of the
//! line, so it may hold spaces and tabs.

use super::*;

#[test]
fn an_unchanged_tab_separated_answer_is_not_written_again() -> Result<(), String> {
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

/// `debconf-get-selections` comma-joins the owner field when a question
/// has several owners, so the entry is read for every owner, not only for
/// a question the package owns alone.
#[test]
fn an_answer_with_several_owners_is_read() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx-common,nginx\tnginx/nginx-user\tstring\twww-data\n".to_owned()),
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
fn a_value_with_spaces_is_read_as_it_is() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www data"}),
        &context(false),
        &mut |_| Ok("nginx\tnginx/nginx-user\tstring\twww data\n".to_owned()),
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
fn the_comments_of_the_tool_are_not_answers() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| {
            Ok(
                "# Choices: www-data, nginx\nnginx\tnginx/nginx-user\tstring\twww-data\n"
                    .to_owned(),
            )
        },
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
fn a_line_that_is_not_tab_separated_is_not_an_answer() -> Result<(), String> {
    let mut written = Vec::new();
    let result = run_with(
        &json!({"package": "nginx", "question": "nginx/nginx-user", "type": "string", "value": "www-data"}),
        &context(false),
        &mut |_| Ok("nginx nginx/nginx-user string www-data\n".to_owned()),
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

#[test]
fn the_answer_of_one_question_is_read_from_the_selections() {
    let selections =
        "nginx\tnginx/nginx-user\tstring\twww-data\npostfix\tpostfix/mailname\tstring\tmail\n";
    let answer = answer_of(selections, "postfix", "postfix/mailname").expect("the answer");
    assert_eq!(answer.kind, "string");
    assert_eq!(answer.value, "mail");
    let answer = answer_of(selections, "nginx", "nginx/nginx-user").expect("the answer");
    assert_eq!(answer.kind, "string");
    assert_eq!(answer.value, "www-data");
    assert!(answer_of(selections, "nginx", "nginx/other").is_none());
    assert!(answer_of(selections, "other", "nginx/nginx-user").is_none());
}

#[test]
fn a_value_with_a_tab_is_read_as_it_is() {
    let selections = "nginx\tnginx/nginx-user\tstring\twww\tdata\n";
    let answer = answer_of(selections, "nginx", "nginx/nginx-user").expect("the answer");
    assert_eq!(answer.kind, "string");
    assert_eq!(answer.value, "www\tdata");
}

#[test]
fn an_empty_value_is_read_as_it_is() {
    let selections = "nginx\tnginx/nginx-user\tstring\t\n";
    let answer = answer_of(selections, "nginx", "nginx/nginx-user").expect("the answer");
    assert_eq!(answer.kind, "string");
    assert_eq!(answer.value, "");
}

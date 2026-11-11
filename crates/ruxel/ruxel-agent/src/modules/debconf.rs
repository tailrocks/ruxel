//! One debconf selection (`debconf-set-selections`).
//!
//! The answer on the host is read with `debconf-get-selections` and set
//! only when it differs, so a second apply reports no change. The answer
//! reaches the host on the standard input of `debconf-set-selections` and
//! never in an argument: an argument is visible in the process list, and a
//! secret must not be.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::io::Write;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_with(params, ctx, &mut current_selections, &mut set_selection)
}

fn run_with(
    params: &Value,
    ctx: &ExecContext,
    current: &mut impl FnMut() -> Result<String, String>,
    set: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let package = str_param(obj, "package").ok_or("debconf: package required")?;
    let question = str_param(obj, "question").ok_or("debconf: question required")?;
    let kind = str_param(obj, "type").ok_or("debconf: type required")?;
    let value = str_param(obj, "value").ok_or("debconf: value required")?;
    if value.contains('\n') || value.contains('\r') {
        return Err(
            "debconf: the answer must be one line; a line break would write a second selection"
                .into(),
        );
    }
    if package.contains('\n') || question.contains('\n') {
        return Err("debconf: the package and the question must be one line".into());
    }
    let answer = current()?;
    let changed = match answer_of(&answer, package, question) {
        Some(answer) => answer.kind != kind || answer.value != value,
        None => true,
    };
    if changed && !ctx.check_mode {
        set(&format!("{package} {question} {kind} {value}\n"))?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

/// One answer of the debconf database: the type of the question and the
/// value the database holds for it.
struct Answer {
    /// The type of the question, as the database knows it.
    kind: String,
    /// The value of the answer.
    value: String,
}

/// The answer the host has for one question, if it has one.
///
/// `debconf-get-selections` prints the database as TAB separated fields,
/// `owner<TAB>question<TAB>type<TAB>value`; the value is the rest of the
/// line, so it may hold spaces and tabs. The lines that begin with `#` are
/// the comments of the tool and carry no answer.
fn answer_of(selections: &str, package: &str, question: &str) -> Option<Answer> {
    for line in selections.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(4, '\t');
        let (Some(owner), Some(name), Some(kind)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if owner == package && name == question {
            return Some(Answer {
                kind: kind.to_owned(),
                value: fields.next().unwrap_or_default().to_owned(),
            });
        }
    }
    None
}

/// The selections the host knows, as `debconf-get-selections` prints them.
fn current_selections() -> Result<String, String> {
    let output = std::process::Command::new("debconf-get-selections")
        .output()
        .map_err(|error| format!("exec debconf-get-selections: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "debconf-get-selections failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One selection, written to the standard input of `debconf-set-selections`.
fn set_selection(selection: &str) -> Result<(), String> {
    let mut child = std::process::Command::new("debconf-set-selections")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("exec debconf-set-selections: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(selection.as_bytes())
            .map_err(|error| format!("write debconf selection: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("exec debconf-set-selections: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "debconf-set-selections failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

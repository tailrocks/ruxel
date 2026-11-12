//! One debconf selection (`debconf-set-selections`).
//!
//! The answer on the host is read with `debconf-get-selections`, which
//! `debconf-utils` ships and `debconf` only suggests; where that optional
//! reader is not installed, `debconf-communicate` (base `debconf`) answers
//! the value of the one question, and the comparison sees the value only.
//! The answer reaches the host on the standard input of
//! `debconf-set-selections` and never in an argument: an argument is
//! visible in the process list, and a secret must not be.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::io::Write;

/// Why the database could not be read with `debconf-get-selections`.
enum ReadError {
    /// The tool is not installed.
    MissingTool,
    /// The tool ran and failed, or could not be executed at all.
    Failed(String),
}

/// What the host answered for one question.
enum Reading {
    /// The answer the database holds: the type of the question, the
    /// value, as `debconf-get-selections` prints them, and whether the
    /// question is marked seen.
    Full(Answer, bool),
    /// The value alone and the seen flag: `debconf-communicate` answers
    /// both; the type is the one of the template, so the comparison sees
    /// the value only.
    Value(Reply),
    /// The host holds no answer for the question.
    Absent,
}

/// What `debconf-communicate` answers for one question: its value and
/// whether the database marks it seen. `debconf-set-selections` marks the
/// questions it writes seen, so an unseen question must be written again
/// even when the answer matches — the next package configuration would
/// otherwise prompt or pick a frontend default.
#[derive(Debug)]
struct Reply {
    value: String,
    seen: bool,
}

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_with(
        params,
        ctx,
        &mut current_selections,
        &mut communicate_value,
        &mut set_selection,
    )
}

fn run_with(
    params: &Value,
    ctx: &ExecContext,
    selections: &mut impl FnMut(&ExecContext) -> Result<String, ReadError>,
    ask: &mut impl FnMut(&str, &str, &ExecContext) -> Result<Option<Reply>, String>,
    set: &mut impl FnMut(&str, &ExecContext) -> Result<(), String>,
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
    if kind == "boolean" && value != "true" && value != "false" {
        // Debconf frontends read a boolean answer as the canonical `true`
        // and `false`; anything else — `yes`, a typo — is accepted by the
        // setter but means nothing, so the package would observe the wrong
        // answer while the task claims convergence.
        return Err(format!(
            "debconf: a boolean answer is \"true\" or \"false\", not {value:?}"
        ));
    }
    if package.contains('\n') || question.contains('\n') {
        return Err("debconf: the package and the question must be one line".into());
    }
    let reading = read_host(selections, ask, package, question, ctx)?;
    let changed = match &reading {
        Reading::Full(answer, seen) => answer.kind != kind || answer.value != value || !seen,
        Reading::Value(reply) => reply.value != value || !reply.seen,
        Reading::Absent => true,
    };
    if changed && !ctx.check_mode {
        set(&format!("{package} {question} {kind} {value}\n"), ctx)?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

/// The answer the host holds for one question: the database where
/// `debconf-get-selections` can read it, the value alone where only
/// `debconf-communicate` (base `debconf`) is installed.
fn read_host(
    selections: &mut impl FnMut(&ExecContext) -> Result<String, ReadError>,
    ask: &mut impl FnMut(&str, &str, &ExecContext) -> Result<Option<Reply>, String>,
    package: &str,
    question: &str,
    ctx: &ExecContext,
) -> Result<Reading, String> {
    match selections(ctx) {
        Ok(text) => match answer_of(&text, package, question) {
            Some(answer) => {
                // The database reader prints the answer but not the seen
                // flag; the fallback reader is asked for it, and a
                // question it cannot describe counts as seen.
                let seen = match ask(package, question, ctx) {
                    Ok(Some(reply)) => reply.seen,
                    _ => true,
                };
                Ok(Reading::Full(answer, seen))
            }
            None => Ok(Reading::Absent),
        },
        Err(ReadError::MissingTool) => Ok(match ask(package, question, ctx)? {
            Some(reply) => Reading::Value(reply),
            None => Reading::Absent,
        }),
        Err(ReadError::Failed(message)) => Err(message),
    }
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
fn current_selections(ctx: &ExecContext) -> Result<String, ReadError> {
    current_selections_named("debconf-get-selections", ctx)
}

/// The selections `program` prints; `program` names the reader in every
/// failure.
fn current_selections_named(program: &str, ctx: &ExecContext) -> Result<String, ReadError> {
    let output = debconf_command(program, ctx)
        .output()
        .map_err(|error| map_reader_exec(program, &error))?;
    if !output.status.success() {
        return Err(ReadError::Failed(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The value and the seen flag the host holds for one question, as
/// `debconf-communicate` answers them: `GET question` answers `0 value`,
/// `GETSEEN question` answers `0 true` or `0 false`, and a question the
/// database does not know answers with a nonzero code.
fn communicate_value(
    _package: &str,
    question: &str,
    ctx: &ExecContext,
) -> Result<Option<Reply>, String> {
    let mut child = debconf_command("debconf-communicate", ctx)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| map_ask_exec(&error))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("GET {question}\nGETSEEN {question}\n").as_bytes())
            .map_err(|error| format!("write debconf-communicate command: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| map_ask_exec(&error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = stdout.lines();
    let value = communicate_reply(
        lines.next().unwrap_or_default(),
        &stderr,
        output.status.success(),
    )?;
    // The seen flag serves the comparison alone: a `GETSEEN` that answers
    // nothing counts as seen, so a read that half worked never invents a
    // change.
    let seen = communicate_reply(
        lines.next().unwrap_or_default(),
        &stderr,
        output.status.success(),
    )
    .ok()
    .flatten()
    .as_deref()
        != Some("false");
    Ok(value.map(|value| Reply { value, seen }))
}

/// What `debconf-communicate` answered: its protocol reply, not its exit
/// status, decides. The tool exits with the code of the last reply, so a
/// question the database does not know answers `10 ... doesn't exist` and
/// exits 10; treating that as a crash would fail a first-time preseed that
/// must simply proceed to `debconf-set-selections`.
fn communicate_reply(
    stdout: &str,
    stderr: &str,
    status_success: bool,
) -> Result<Option<String>, String> {
    let reply = stdout.lines().next().unwrap_or_default();
    let (code, text) = reply.split_once(' ').unwrap_or((reply, ""));
    if !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) {
        return match code {
            "0" => Ok(Some(text.to_owned())),
            _ => Ok(None),
        };
    }
    if !status_success {
        return Err(format!("debconf-communicate failed: {}", stderr.trim_end()));
    }
    Err(format!("debconf-communicate: unexpected reply {reply:?}"))
}

/// One selection, written to the standard input of `debconf-set-selections`.
fn set_selection(selection: &str, ctx: &ExecContext) -> Result<(), String> {
    set_selection_named("debconf-set-selections", selection, ctx)
}

/// One selection, written to the standard input of `program`; `program`
/// names the writer in every failure.
fn set_selection_named(program: &str, selection: &str, ctx: &ExecContext) -> Result<(), String> {
    let mut child = debconf_command(program, ctx)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("exec {program}: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(selection.as_bytes())
            .map_err(|error| format!("write debconf selection: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("exec {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    Ok(())
}

/// A `debconf-*` command, under the task environment: `DEBCONF_DB_OVERRIDE`
/// and `DEBCONF_DB_FALLBACK` steer which database the tools touch, and a
/// command that ignored the environment would silently answer and write
/// the default one.
fn debconf_command(program: &str, ctx: &ExecContext) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    for (key, value) in &ctx.environment {
        command.env(key, value);
    }
    command
}

fn map_reader_exec(program: &str, error: &std::io::Error) -> ReadError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ReadError::MissingTool
    } else {
        ReadError::Failed(format!("exec {program}: {error}"))
    }
}

fn map_ask_exec(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "debconf: debconf-communicate is not installed; install the debconf package".to_owned()
    } else {
        format!("exec debconf-communicate: {error}")
    }
}

#[cfg(test)]
mod tests;

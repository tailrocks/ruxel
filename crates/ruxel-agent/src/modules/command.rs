//! `command` (SEMANTICS §6): argv exec, no shell. Free-form bodies split
//! shlex-style (pinned by golden E15: `echo 'a b' c` → ["echo","a b","c"]).
//! Always changed (the controller's changed_when normally overrides);
//! failure = rc≠0.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, free_form: &str, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let argv: Vec<String> = if !free_form.is_empty() {
        shlex_split(free_form)?
    } else if let Some(cmd) = str_param(obj, "cmd") {
        shlex_split(cmd)?
    } else if let Some(list) = obj.get("argv").and_then(Value::as_array) {
        list.iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "argv entries must be strings".to_string())
            })
            .collect::<Result<_, _>>()?
    } else {
        return Err("command needs a free-form body, cmd, or argv".into());
    };
    if argv.is_empty() {
        return Err("empty command".into());
    }

    if let Some(result) = creates_guard(obj, Value::from(argv.clone())) {
        return Ok(result);
    }

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut cmd = super::become_command(ctx, argv_refs[0], &argv_refs[1..]);
    if let Some(chdir) = str_param(obj, "chdir") {
        cmd.current_dir(chdir);
    }
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    let output = cmd.output().map_err(|e| format!("exec {}: {e}", argv[0]))?;
    Ok(command_result(Value::from(argv), &output))
}

pub(super) fn creates_guard(params: &serde_json::Map<String, Value>, cmd: Value) -> Option<Value> {
    let creates = str_param(params, "creates")?;
    std::path::Path::new(creates).exists().then(|| {
        json!({
            "cmd": cmd,
            "rc": 0,
            "changed": false,
            "failed": false,
            "msg": format!("Did not run command since '{creates}' exists"),
            "stdout": format!("skipped, since {creates} exists"),
            "stderr": "",
            "stdout_lines": [format!("skipped, since {creates} exists")],
            "stderr_lines": [],
            "start": null,
            "end": null,
            "delta": null,
        })
    })
}

/// Result fields shared with shell (SEMANTICS §3.8).
pub fn command_result(cmd: Value, output: &std::process::Output) -> Value {
    let rc = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .trim_end_matches('\n')
        .to_string();
    let lines = |s: &str| -> Vec<String> {
        if s.is_empty() {
            vec![]
        } else {
            s.lines().map(str::to_string).collect()
        }
    };
    json!({
        "cmd": cmd,
        "rc": rc,
        "stdout": stdout,
        "stderr": stderr,
        "stdout_lines": lines(&stdout),
        "stderr_lines": lines(&stderr),
        "changed": true,
        "failed": rc != 0,
        "msg": if rc != 0 { "The command exited with a non-zero return code." } else { "" },
    })
}

/// POSIX-shlex split, matching Python shlex.split for the shapes the
/// workload uses (plain words, single/double quotes, backslash escapes).
fn shlex_split(input: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                let mut closed = false;
                for q in chars.by_ref() {
                    if q == '\'' {
                        closed = true;
                        break;
                    }
                    current.push(q);
                }
                if !closed {
                    return Err("No closing quotation".into());
                }
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => {
                            if let Some(&n) = chars.peek()
                                && (n == '"' || n == '\\' || n == '$' || n == '`')
                            {
                                current.push(n);
                                chars.next();
                            } else {
                                current.push('\\');
                            }
                        }
                        other => current.push(other),
                    }
                }
                if !closed {
                    return Err("No closing quotation".into());
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        out.push(current);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn shlex_matches_golden_e15() {
        assert_eq!(
            shlex_split("echo 'a b' c").unwrap(),
            vec!["echo", "a b", "c"]
        );
    }

    #[test]
    fn shlex_double_quotes_and_escapes() {
        assert_eq!(
            shlex_split(r#"sh -c "echo \"x\" y""#).unwrap(),
            vec!["sh", "-c", r#"echo "x" y"#]
        );
    }

    #[test]
    fn shlex_rejects_unclosed_quotes() {
        assert_eq!(
            shlex_split("echo 'unterminated").unwrap_err(),
            "No closing quotation"
        );
        assert_eq!(
            shlex_split("echo \"unterminated").unwrap_err(),
            "No closing quotation"
        );
    }

    #[test]
    fn result_shape_tracks_rc_and_output_lines() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(7 << 8),
            stdout: b"one\ntwo\n".to_vec(),
            stderr: b"bad\n".to_vec(),
        };
        let result = command_result(json!(["synthetic"]), &output);
        assert_eq!(result["rc"], 7);
        assert_eq!(result["failed"], true);
        assert_eq!(result["stdout_lines"], json!(["one", "two"]));
        assert_eq!(result["stderr_lines"], json!(["bad"]));
    }

    #[test]
    fn creates_guard_matches_ansible_skip_shape() {
        let marker =
            std::env::temp_dir().join(format!("ruxel-command-creates-{}", std::process::id()));
        std::fs::write(&marker, b"").unwrap();
        let params = json!({"creates": marker});
        let result = creates_guard(
            params.as_object().unwrap(),
            json!(["/usr/bin/touch", "synthetic"]),
        )
        .unwrap();
        std::fs::remove_file(marker).unwrap();

        assert_eq!(result["changed"], false);
        assert_eq!(result["rc"], 0);
        assert_eq!(result["cmd"], json!(["/usr/bin/touch", "synthetic"]));
        assert!(
            result["stdout"]
                .as_str()
                .unwrap()
                .starts_with("skipped, since ")
        );
        assert!(result["start"].is_null());
        assert!(result["delta"].is_null());
    }
}

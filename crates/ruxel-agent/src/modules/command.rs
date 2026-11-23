#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(
    clippy::needless_pass_by_value,
    reason = "replaced by the Ruxel rewrite"
)]

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    if obj.contains_key("cmd") {
        return Err("command accepts argv only".into());
    }
    let argv: Vec<String> = obj
        .get("argv")
        .and_then(Value::as_array)
        .ok_or_else(|| "command requires argv".to_string())?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| "argv entries must be strings".to_string())
        })
        .collect::<Result<_, _>>()?;
    if argv.first().is_none_or(String::is_empty) {
        return Err("empty command".into());
    }

    validate_become_options(obj, ctx)?;
    if let Some(result) = creates_guard(obj, Value::from(argv.clone())) {
        return Ok(result);
    }
    if let Some(result) = check_mode_guard(obj, Value::from(argv.clone()), ctx.check_mode) {
        return Ok(result);
    }

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut cmd = super::become_command(ctx, argv_refs[0], &argv_refs[1..])?;
    if let Some(chdir) = str_param(obj, "chdir") {
        cmd.current_dir(chdir);
    }
    let output = cmd.output().map_err(|e| format!("exec {}: {e}", argv[0]))?;
    Ok(command_result(Value::from(argv), &output))
}

pub(super) fn validate_become_options(
    params: &serde_json::Map<String, Value>,
    ctx: &ExecContext,
) -> Result<(), String> {
    if ctx.become_user.is_some() {
        for option in ["creates", "chdir"] {
            if str_param(params, option).is_some() {
                return Err(format!("{option} is not supported with become_user"));
            }
        }
    }
    Ok(())
}

/// Re-stat `creates` immediately before the command runs. The controller's
/// own probe gates the task before it resolves the environment — a task it
/// skips reads no secret store — but its answer ages: a marker created after
/// it would let the command run anyway, a removed one would keep a stale
/// skip. The agent checks the path itself now, at execution: it exists by
/// then, the command does not run and nothing reports a change; it vanished,
/// the dispatched command runs.
///
/// `run_if_changed` is the `onChangeOf` guard the controller evaluated when
/// it dispatched: a watched step changed, so the created path is stale and
/// the command runs even where it exists, exactly as the task's `when` says.
/// The same `creates_exists` cause tells the engine's `changedWhen` that this
/// skip reports no change, where a check-mode skip reports a pending one.
pub(super) fn creates_guard(params: &serde_json::Map<String, Value>, cmd: Value) -> Option<Value> {
    let creates = str_param(params, "creates")?;
    if super::bool_param(params, "run_if_changed", false) {
        return None;
    }
    // Follows links, as the controller's probe does: a dangling link is not
    // the path existing.
    std::path::Path::new(creates).exists().then(|| {
        json!({
            "cmd": cmd, "rc": 0, "changed": false, "failed": false,
            "skipped": true,
            "skip": {"cause": "creates_exists"},
            "msg": format!("Did not run command since '{creates}' exists"),
            "stdout": "", "stderr": "", "stdout_lines": [], "stderr_lines": [],
        })
    })
}

/// What a command reports when the mode says it must not run.
///
/// A command with `creates` would run and would change something: the
/// marker is absent (a marker that is present is skipped by
/// `creates_guard`, which reports no change), so the check-mode skip is a
/// pending change and the task says so. Every other command reports no
/// change: what it would do is unknown, and a task that only inspects
/// changes nothing.
///
/// The report is the task's own: a task that carries an explicit
/// `changedWhen` is judged by that expression, not by this one.
pub(super) fn check_mode_guard(
    params: &serde_json::Map<String, Value>,
    cmd: Value,
    check_mode: bool,
) -> Option<Value> {
    check_mode.then(|| {
        if str_param(params, "creates").is_some() {
            json!({
                "cmd": cmd, "rc": 0, "changed": true, "failed": false,
                "msg": "Command would have run if not in check mode",
                "stdout": "", "stderr": "", "stdout_lines": [], "stderr_lines": [],
            })
        } else {
            json!({
                "cmd": cmd, "rc": 0, "changed": false, "failed": false,
                "skipped": true,
                "msg": "Command would have run if not in check mode",
                "stdout": "", "stderr": "", "stdout_lines": [], "stderr_lines": [],
            })
        }
    })
}

pub(super) fn command_result(cmd: Value, output: &std::process::Output) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn argv_preserves_literal_arguments_and_rejects_legacy_cmd() {
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let result = run(
            &json!({"argv":["/usr/bin/printf", "%s", "a b; $HOME 'quoted'"]}),
            &ctx,
        )
        .unwrap();
        assert_eq!(result["stdout"], "a b; $HOME 'quoted'");
        assert!(run(&json!({"cmd":"printf legacy"}), &ctx).is_err());
        assert!(run(&json!({"argv":[], "cmd":"legacy"}), &ctx).is_err());
    }

    /// The controller probed the created path before it resolved the task's
    /// environment and dispatched the command; the marker appears only now,
    /// between that dispatch and the exec. The agent re-stats the path itself:
    /// the command does not run, and nothing reports a change. The converse
    /// holds too — a marker that vanished since the probe lets the dispatched
    /// command run.
    #[test]
    fn creates_recheck_skips_a_marker_that_appeared_after_dispatch() {
        let root =
            std::env::temp_dir().join(format!("ruxel-command-creates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("marker");
        let executed = root.join("executed");
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };

        // The probe saw no marker, so the controller dispatched the command.
        assert!(!marker.exists());
        std::fs::write(&marker, b"created after the probe").unwrap();
        let params = json!({"argv": ["/usr/bin/touch", executed], "creates": marker});
        let result = run(&params, &ctx).unwrap();
        assert_eq!(result["skipped"], true, "{result}");
        assert_eq!(result["changed"], false, "{result}");
        assert_eq!(result["failed"], false, "{result}");
        assert_eq!(result["skip"]["cause"], "creates_exists", "{result}");
        assert!(!executed.exists(), "the command must not run");

        // In check and diff the same recheck reports nothing to do either.
        let check = ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let result = run(&params, &check).unwrap();
        assert_eq!(result["skipped"], true, "{result}");
        assert_eq!(result["changed"], false, "{result}");
        assert!(!executed.exists(), "the command must not run");

        // The marker vanished after the probe: the dispatched command runs.
        std::fs::remove_file(&marker).unwrap();
        let result = run(&params, &ctx).unwrap();
        assert_eq!(result["changed"], true, "{result}");
        assert!(executed.exists(), "the command must run");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A command with `creates` reports the pending change in check mode:
    /// the marker is absent, so the command would run and would change
    /// something. The marker is present, the command is skipped by the
    /// `creates` guard, and nothing reports a change.
    #[test]
    fn a_creates_command_in_check_mode_reports_the_pending_change() {
        let root = std::env::temp_dir().join(format!("ruxel-command-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("marker");
        let executed = root.join("executed");
        let params = json!({"argv": ["/usr/bin/touch", executed], "creates": marker});
        let check = ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };

        // The marker is absent: the command would run, so the check-mode
        // report is a pending change.
        let result = run(&params, &check).unwrap();
        assert_eq!(result["changed"], true, "{result}");
        assert_eq!(result["failed"], false, "{result}");
        assert!(!executed.exists(), "the command must not run");

        // The marker is present: the `creates` guard skips the command and
        // reports no change.
        std::fs::write(&marker, b"present").unwrap();
        let result = run(&params, &check).unwrap();
        assert_eq!(result["skipped"], true, "{result}");
        assert_eq!(result["changed"], false, "{result}");
        assert_eq!(result["skip"]["cause"], "creates_exists", "{result}");
        assert!(!executed.exists(), "the command must not run");
        let _ = std::fs::remove_dir_all(root);
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
}

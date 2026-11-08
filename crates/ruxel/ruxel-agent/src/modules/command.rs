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

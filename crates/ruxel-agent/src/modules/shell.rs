#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::command::{check_mode_guard, command_result, creates_guard, validate_become_options};
use super::{ExecContext, params_object, str_param};
use serde_json::Value;

pub(super) fn run(params: &Value, free_form: &str, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    if free_form.is_empty() {
        return Err("shell needs a free-form body".into());
    }

    validate_become_options(obj, ctx)?;
    if let Some(result) = creates_guard(obj, Value::from(free_form)) {
        return Ok(result);
    }
    if let Some(result) = check_mode_guard(obj, Value::from(free_form), ctx.check_mode) {
        return Ok(result);
    }

    let executable = str_param(obj, "executable").unwrap_or("/bin/sh");
    let mut cmd = super::become_command(ctx, executable, &["-c", free_form])?;
    if let Some(chdir) = str_param(obj, "chdir") {
        cmd.current_dir(chdir);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("exec {executable}: {e}"))?;
    Ok(command_result(Value::from(free_form), &output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The script's own recheck of `creates`, at execution: the controller
    /// probed the path, found it missing and dispatched the script, and the
    /// marker only appears now. The script does not run, the skip reports no
    /// change, and a marker that vanished lets the dispatched script run.
    #[test]
    fn creates_recheck_skips_a_marker_that_appeared_after_dispatch() {
        let root = std::env::temp_dir().join(format!("ruxel-shell-creates-{}", std::process::id()));
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
        let params = json!({"creates": marker});
        let script = format!("touch {}", executed.display());

        assert!(!marker.exists());
        std::fs::write(&marker, b"created after the probe").unwrap();
        let result = run(&params, &script, &ctx).unwrap();
        assert_eq!(result["skipped"], true, "{result}");
        assert_eq!(result["changed"], false, "{result}");
        assert_eq!(result["skip"]["cause"], "creates_exists", "{result}");
        assert!(!executed.exists(), "the script must not run");

        std::fs::remove_file(&marker).unwrap();
        let result = run(&params, &script, &ctx).unwrap();
        assert_eq!(result["changed"], true, "{result}");
        assert!(executed.exists(), "the script must run");
        let _ = std::fs::remove_dir_all(root);
    }
}

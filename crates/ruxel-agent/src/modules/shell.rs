//! `shell` (SEMANTICS §6): free-form via the shell, with the workload's
//! args (executable, chdir, creates). The creates-guard result shape is
//! pinned by golden E14: status ok, changed false, rc 0, the
//! "Did not run command" msg, and null timing fields.

use super::command::{check_mode_guard, command_result, creates_guard};
use super::{ExecContext, params_object, str_param};
use serde_json::Value;

pub fn run(params: &Value, free_form: &str, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    if free_form.is_empty() {
        return Err("shell needs a free-form body".into());
    }

    if let Some(result) = creates_guard(obj, Value::from(free_form)) {
        return Ok(result);
    }
    if let Some(result) = check_mode_guard(obj, Value::from(free_form), ctx.check_mode) {
        return Ok(result);
    }

    let executable = str_param(obj, "executable").unwrap_or("/bin/sh");
    let mut cmd = super::become_command(ctx, executable, &["-c", free_form]);
    if let Some(chdir) = str_param(obj, "chdir") {
        cmd.current_dir(chdir);
    }
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("exec {executable}: {e}"))?;
    Ok(command_result(Value::from(free_form), &output))
}

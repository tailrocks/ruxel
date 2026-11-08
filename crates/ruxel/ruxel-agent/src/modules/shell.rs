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

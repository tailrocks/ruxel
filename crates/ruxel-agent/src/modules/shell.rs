//! `shell` (SEMANTICS §6): free-form via the shell, with the workload's
//! args (executable, chdir, creates). The creates-guard result shape is
//! pinned by golden E14: status ok, changed false, rc 0, the
//! "Did not run command" msg, and null timing fields.

use super::command::command_result;
use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, free_form: &str, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    if free_form.is_empty() {
        return Err("shell needs a free-form body".into());
    }

    if let Some(creates) = str_param(obj, "creates")
        && std::path::Path::new(creates).exists()
    {
        return Ok(json!({
            "cmd": free_form,
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
        }));
    }

    let executable = str_param(obj, "executable").unwrap_or("/bin/sh");
    let mut cmd = std::process::Command::new(executable);
    cmd.arg("-c").arg(free_form);
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

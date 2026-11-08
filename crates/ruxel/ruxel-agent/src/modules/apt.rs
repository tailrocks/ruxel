use super::{ExecContext, bool_param, params_object, str_param};
use crate::system_state::SystemState;
use serde_json::{Map, Value, json};

type CommandResult = Result<(String, i32, String), String>;

pub(super) fn run(
    params: &Value,
    ctx: &ExecContext,
    system_state: &mut SystemState,
) -> Result<Value, String> {
    run_with(params, ctx, system_state, &mut apt_get)
}

fn run_with(
    params: &Value,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let update_cache = bool_param(obj, "update_cache", false);
    let upgrade = str_param(obj, "upgrade");
    let autoremove = bool_param(obj, "autoremove", false);
    let names = package_names(obj)?;
    let state = str_param(obj, "state").unwrap_or("present");
    let mut result = json!({"changed": false});
    let mut changed = false;
    if update_cache {
        if !ctx.check_mode {
            let out = command(&["update"], ctx);
            system_state.invalidate_packages();
            let out = match out {
                Ok(out) => out,
                Err(error) => return Ok(json!({"failed": true, "changed": true, "msg": error})),
            };
            if out.1 != 0 {
                return Ok(
                    json!({"failed": true, "changed": true, "msg": format!("apt update failed: {}", out.2)}),
                );
            }
            changed = true;
        }
        result["cache_updated"] = json!(!ctx.check_mode);
        result["cache_update_time"] = json!(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        );
    }
    if let Some(kind) = upgrade {
        let action = match kind {
            "dist" => "dist-upgrade",
            other => return Err(format!("apt: upgrade={other:?} outside the closed surface")),
        };
        changed |= maintenance(action, true, ctx, system_state, &mut result, command)?;
    }
    if autoremove {
        changed |= maintenance("autoremove", false, ctx, system_state, &mut result, command)?;
        result["diff"] = json!({});
    }
    if !names.is_empty() {
        changed |= install(&names, state, obj, ctx, system_state, &mut result, command)?;
    }
    result["changed"] = json!(changed);
    result["failed"] = json!(false);
    Ok(result)
}

fn package_names(obj: &Map<String, Value>) -> Result<Vec<String>, String> {
    match obj.get("name") {
        None => Ok(vec![]),
        Some(Value::String(name)) => Ok(vec![name.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "apt: name entries must be strings".to_string())
            })
            .collect(),
        Some(other) => Err(format!("apt: invalid name {other:?}")),
    }
}

fn maintenance(
    action: &str,
    with_msg: bool,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<bool, String> {
    let args = if ctx.check_mode {
        vec!["-s", action]
    } else {
        vec![action]
    };
    let (stdout, rc, stderr) = command(&args, ctx)?;
    if rc != 0 {
        let simulation = if ctx.check_mode { " -s" } else { "" };
        return Err(format!("apt {action}{simulation} failed: {stderr}"));
    }
    if !ctx.check_mode {
        system_state.invalidate_packages();
    }
    fill_exec_fields(result, &stdout, &stderr, with_msg);
    Ok(summary_changed(&stdout))
}

fn install(
    names: &[String],
    state: &str,
    obj: &Map<String, Value>,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<bool, String> {
    let missing = system_state.missing_packages(names, state == "latest")?;
    if missing.is_empty() {
        return Ok(false);
    }
    if !ctx.check_mode {
        let mut args = vec!["install"];
        if bool_param(obj, "force", false) {
            args.push("--allow-downgrades");
        }
        args.extend(missing.iter().map(String::as_str));
        args.extend(
            names
                .iter()
                .filter(|name| name.contains('=') && !missing.contains(name))
                .map(String::as_str),
        );
        let (stdout, rc, stderr) = command(&args, ctx)?;
        if rc != 0 {
            return Err(format!("apt install failed: {stderr}"));
        }
        system_state.invalidate_packages();
        let pinned: Vec<_> = names
            .iter()
            .filter(|name| name.contains('='))
            .cloned()
            .collect();
        if !pinned.is_empty() {
            let unresolved = system_state.missing_packages(&pinned, false)?;
            if !unresolved.is_empty() {
                return Err(format!(
                    "apt install did not install the requested versions: {}",
                    unresolved.join(", ")
                ));
            }
        }
        fill_exec_fields(result, &stdout, &stderr, false);
    }
    Ok(true)
}

fn apt_get(args: &[&str], ctx: &ExecContext) -> CommandResult {
    let mut cmd = std::process::Command::new("apt-get");
    cmd.arg("-y")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confdef")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confold");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| format!("exec apt-get: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn summary_changed(stdout: &str) -> bool {
    for line in stdout.lines() {
        if line.contains("upgraded,") && line.contains("newly installed") {
            let numbers: Vec<u64> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            return numbers.iter().take(3).any(|n| *n > 0);
        }
    }
    false
}

fn fill_exec_fields(result: &mut Value, stdout: &str, stderr: &str, with_msg: bool) {
    let stdout = stdout.trim_end_matches('\n');
    let stderr = stderr.trim_end_matches('\n');
    let lines = |s: &str| -> Vec<String> {
        if s.is_empty() {
            vec![]
        } else {
            s.lines().map(str::to_string).collect()
        }
    };
    if with_msg {
        result["msg"] = json!(stdout);
    }
    result["stdout"] = json!(stdout);
    result["stderr"] = json!(stderr);
    result["stdout_lines"] = json!(lines(stdout));
    result["stderr_lines"] = json!(lines(stderr));
}

#[cfg(test)]
mod tests;

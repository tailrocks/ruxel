//! Small closed-surface modules: `community.general.timezone` and `group`.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

/// `community.general.timezone` (1 use: name=UTC) via timedatectl.
pub fn timezone(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("timezone: name required")?;
    let current = std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            p.to_string_lossy()
                .split("/zoneinfo/")
                .nth(1)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let changed = current != name;
    if changed && !ctx.check_mode {
        let out = std::process::Command::new("timedatectl")
            .arg("set-timezone")
            .arg(name)
            .output()
            .map_err(|e| format!("timedatectl: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "timedatectl set-timezone {name}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(json!({"changed": changed, "failed": false}))
}

/// `group` (SEMANTICS §6): name, gid, state via getent/groupadd/groupdel.
pub fn group(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("group: name required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    let gid = obj.get("gid").and_then(Value::as_u64);

    let entry = group_entry(name)?;
    let mut changed = false;

    match state {
        "present" => match entry {
            Some((_, current_gid)) => {
                if let Some(want) = gid
                    && current_gid != want
                {
                    changed = true;
                    if !ctx.check_mode {
                        run_cmd("groupmod", &["-g", &want.to_string(), name])?;
                    }
                }
            }
            None => {
                changed = true;
                if !ctx.check_mode {
                    let gid_s;
                    let mut args: Vec<&str> = Vec::new();
                    if let Some(want) = gid {
                        gid_s = want.to_string();
                        args.push("-g");
                        args.push(&gid_s);
                    }
                    args.push(name);
                    run_cmd("groupadd", &args)?;
                }
            }
        },
        "absent" => {
            if entry.is_some() {
                changed = true;
                if !ctx.check_mode {
                    run_cmd("groupdel", &[name])?;
                }
            }
        }
        other => return Err(format!("group: state {other:?} outside the closed surface")),
    }
    Ok(json!({"changed": changed, "failed": false, "name": name}))
}

fn group_entry(name: &str) -> Result<Option<(String, u64)>, String> {
    let content = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    for line in content.lines() {
        let mut f = line.split(':');
        if f.next() == Some(name) {
            let gid = f
                .nth(1)
                .and_then(|g| g.parse().ok())
                .ok_or("malformed group line")?;
            return Ok(Some((name.to_string(), gid)));
        }
    }
    Ok(None)
}

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("exec {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

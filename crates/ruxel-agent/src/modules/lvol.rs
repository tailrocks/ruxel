use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let vg = str_param(obj, "vg").ok_or("lvol: vg required")?;
    let lv = str_param(obj, "lv").ok_or("lvol: lv required")?;
    let size = str_param(obj, "size").ok_or("lvol: size required")?;
    let resizefs = bool_param(obj, "resizefs", false);

    let exists = lv_exists(vg, lv)?;
    let mut changed = false;

    if !exists {
        changed = true;
        if !ctx.check_mode {
            create(vg, lv, size)?;
            if resizefs {
                run_cmd("lvextend", &["-r", &lv_path(vg, lv)]).ok();
            }
        }
    } else {
        let is_extend = size.starts_with('+');
        if is_extend && free_extents(vg)? > 0 {
            changed = true;
            if !ctx.check_mode {
                let mut args = vec!["-l".to_string(), size.to_string()];
                if resizefs {
                    args.insert(0, "-r".to_string());
                }
                args.push(lv_path(vg, lv));
                let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();

                match run_cmd("lvextend", &args_ref) {
                    Ok(()) => {}
                    Err(e) if e.contains("matches existing size") || e.contains("not larger") => {
                        changed = false;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "lv": lv, "vg": vg}))
}

fn lv_path(vg: &str, lv: &str) -> String {
    format!("/dev/{vg}/{lv}")
}

fn lv_exists(vg: &str, lv: &str) -> Result<bool, String> {
    let out = std::process::Command::new("lvs")
        .args(["--noheadings", "-o", "lv_name", &format!("{vg}/{lv}")])
        .output()
        .map_err(|e| format!("exec lvs: {e}"))?;
    Ok(out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

fn free_extents(vg: &str) -> Result<u64, String> {
    let out = std::process::Command::new("vgs")
        .args(["--noheadings", "-o", "vg_free_count", vg])
        .output()
        .map_err(|e| format!("exec vgs: {e}"))?;
    if !out.status.success() {
        return Ok(0);
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0))
}

fn create(vg: &str, lv: &str, size: &str) -> Result<(), String> {
    let flag = if size.contains('%') { "-l" } else { "-L" };
    run_cmd("lvcreate", &[flag, lvcreate_size(size), "-n", lv, vg])
}

fn lvcreate_size(size: &str) -> &str {
    size.strip_prefix('+').unwrap_or(size)
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_size_removes_relative_prefix() {
        assert_eq!(super::lvcreate_size("+100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("10G"), "10G");
    }
}

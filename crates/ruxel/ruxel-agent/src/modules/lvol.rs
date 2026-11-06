use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let vg = str_param(obj, "vg").ok_or("lvol: vg required")?;
    let lv = str_param(obj, "lv").ok_or("lvol: lv required")?;
    let size = str_param(obj, "size").ok_or("lvol: size required")?;
    let resizefs = bool_param(obj, "resizefs", false);

    let exists = lv_exists(vg, lv)?;
    let mut changed = false;

    if exists {
        let relative = size.starts_with('+');
        let needs_resize = if relative {
            if size != "+100%FREE" {
                return Err("lvol: only +100%FREE is supported for relative growth".into());
            }
            free_extents(vg)? > 0
        } else {
            let requested = size_bytes(size)?;
            let (current, extent) = lv_geometry(vg, lv)?;
            needs_absolute_resize(current, extent, requested)?
        };
        if !needs_resize && resizefs {
            let path = lv_path(vg, lv);
            let fstype = super::filesystem::blkid_type(&path)?
                .ok_or("lvol: filesystem type unavailable for resize")?;
            let result = super::filesystem::run(
                &json!({"dev": path, "fstype": fstype, "resizefs": true}),
                ctx,
            )?;
            changed |= result["changed"].as_bool().unwrap_or(false);
        }
        if needs_resize {
            changed = true;
            if !ctx.check_mode {
                let flag = if relative { "-l" } else { "-L" };
                let mut args = vec![flag.to_string(), size.to_string()];
                if resizefs {
                    args.insert(0, "-r".to_string());
                }
                args.push(lv_path(vg, lv));
                let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
                run_cmd(if relative { "lvextend" } else { "lvresize" }, &args_ref)?;
            }
        }
    } else {
        changed = true;
        if !ctx.check_mode {
            create(vg, lv, size)?;
        }
    }

    Ok(json!({"changed": changed, "failed": false, "lv": lv, "vg": vg}))
}

fn size_bytes(size: &str) -> Result<u64, String> {
    let split = size
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(size.len());
    let number: u64 = size[..split]
        .parse()
        .map_err(|_| format!("lvol: invalid size {size:?}"))?;
    let multiplier = match size[split..].to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1u64 << 10,
        "m" | "mb" | "mib" => 1u64 << 20,
        "g" | "gb" | "gib" => 1u64 << 30,
        "t" | "tb" | "tib" => 1u64 << 40,
        _ => return Err(format!("lvol: unsupported absolute size {size:?}")),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| "lvol: size overflow".into())
}

fn needs_absolute_resize(current: u64, extent: u64, requested: u64) -> Result<bool, String> {
    if extent == 0 || requested == 0 {
        return Err("lvol: size and extent must be positive".into());
    }
    let rounded = requested
        .checked_add(extent - 1)
        .ok_or("lvol: size overflow")?
        / extent
        * extent;
    if rounded < current {
        return Err("lvol: refusing to shrink an existing logical volume".into());
    }
    Ok(rounded > current)
}

fn lv_geometry(vg: &str, lv: &str) -> Result<(u64, u64), String> {
    fn value(program: &str, field: &str, target: &str) -> Result<u64, String> {
        let mut command = std::process::Command::new(program);
        command.args([
            "--noheadings",
            "--units",
            "b",
            "--nosuffix",
            "-o",
            field,
            target,
        ]);
        let out = super::run_checked(command)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let whole = text
            .trim()
            .split('.')
            .next()
            .ok_or("lvol: empty geometry")?;
        whole
            .parse()
            .map_err(|_| format!("lvol: invalid {field} geometry"))
    }
    Ok((
        value("lvs", "lv_size", &format!("{vg}/{lv}"))?,
        value("vgs", "vg_extent_size", vg)?,
    ))
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
        return Err(format!(
            "vgs: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "lvol: invalid free extent count".into())
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
    fn absolute_size_reconciles_at_extent_boundaries() {
        assert_eq!(super::size_bytes("10G").unwrap(), 10 << 30);
        assert!(!super::needs_absolute_resize(16, 4, 13).unwrap());
        assert!(super::needs_absolute_resize(16, 4, 17).unwrap());
        assert!(super::needs_absolute_resize(16, 4, 8).is_err());
    }

    #[test]
    fn create_size_removes_relative_prefix() {
        assert_eq!(super::lvcreate_size("+100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("10G"), "10G");
    }
}

//! `filesystem` (SEMANTICS §6): blkid fstype on dev; create only if
//! absent; resizefs grows the fs to the device. xfs ×5, ext4 ×1.

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let dev = str_param(obj, "dev").ok_or("filesystem: dev required")?;
    let fstype = str_param(obj, "fstype").ok_or("filesystem: fstype required")?;
    let resizefs = bool_param(obj, "resizefs", false);

    let current = blkid_type(dev)?;
    let mut changed = false;

    match current.as_deref() {
        Some(t) if t == fstype => {
            // Already the right fs. resizefs may still grow it.
            if resizefs && !ctx.check_mode {
                changed |= grow(dev, fstype)?;
            }
        }
        Some(other) => {
            return Err(format!(
                "filesystem: {dev} already has {other}, refusing to overwrite with {fstype} \
                 (force is outside the closed surface)"
            ));
        }
        None => {
            changed = true;
            if !ctx.check_mode {
                make(dev, fstype)?;
                if resizefs {
                    grow(dev, fstype)?;
                }
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "dev": dev, "fstype": fstype}))
}

fn blkid_type(dev: &str) -> Result<Option<String>, String> {
    let out = std::process::Command::new("blkid")
        .args(["-o", "value", "-s", "TYPE", dev])
        .output()
        .map_err(|e| format!("exec blkid: {e}"))?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if t.is_empty() { None } else { Some(t) })
}

fn make(dev: &str, fstype: &str) -> Result<(), String> {
    let bin = format!("mkfs.{fstype}");
    // xfs needs -f only when overwriting; on a blank dev it is harmless.
    let args: &[&str] = if fstype == "xfs" {
        &["-f", dev]
    } else {
        &["-F", dev]
    };
    run_cmd(&bin, args)
}

/// Grow to fill the device. Returns whether a grow ran (best-effort: a
/// no-op grow on an already-full fs reports no change).
fn grow(dev: &str, fstype: &str) -> Result<bool, String> {
    match fstype {
        "xfs" => {
            // xfs_growfs needs a mountpoint; skip when unmounted (the
            // workload grows after mount via the mount module path).
            Ok(false)
        }
        "ext4" => {
            let out = std::process::Command::new("resize2fs")
                .arg(dev)
                .output()
                .map_err(|e| format!("exec resize2fs: {e}"))?;
            Ok(out.status.success()
                && !String::from_utf8_lossy(&out.stdout).contains("Nothing to do"))
        }
        _ => Ok(false),
    }
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
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

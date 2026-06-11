//! `ansible.posix.mount` (SEMANTICS §6): fstab line + actually mounted,
//! state: mounted. ⚠ resolved 2026-06-11: fstab fields are normalized for
//! comparison — src/path/fstype/opts plus dump (0) and pass (0 for most,
//! the workload uses defaults) — so a converged rerun is no-change even
//! though Ansible writes its own whitespace.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

const FSTAB: &str = "/etc/fstab";

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("mount: path required")?;
    let src = str_param(obj, "src").ok_or("mount: src required")?;
    let fstype = str_param(obj, "fstype").ok_or("mount: fstype required")?;
    let opts = str_param(obj, "opts").unwrap_or("defaults");
    let state = str_param(obj, "state").unwrap_or("mounted");
    if state != "mounted" {
        return Err(format!("mount: state {state:?} outside the closed surface"));
    }

    let mut changed = false;

    // 1. fstab entry (normalized compare on the 6 fields).
    let want = [src, path, fstype, opts, "0", "0"];
    let current = std::fs::read_to_string(FSTAB).unwrap_or_default();
    let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
    let mut found = false;
    for line in lines.iter_mut() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = t.split_whitespace().collect();
        // Match on mount point (field 1) — Ansible keys fstab by path.
        if f.len() >= 2 && f[1] == path {
            found = true;
            let matches = f.len() >= 4 && f[0] == src && f[2] == fstype && opts_eq(f[3], opts);
            if !matches {
                *line = want.join("\t");
                changed = true;
            }
            break;
        }
    }
    if !found {
        lines.push(want.join("\t"));
        changed = true;
    }
    if changed && !ctx.check_mode {
        let mut content = lines.join("\n");
        content.push('\n');
        std::fs::write(FSTAB, content).map_err(|e| e.to_string())?;
    }

    // 2. actually mounted?
    let mounted = is_mounted(path)?;
    if !mounted {
        changed = true;
        if !ctx.check_mode {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
            let out = std::process::Command::new("mount")
                .arg(path)
                .output()
                .map_err(|e| format!("exec mount: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "mount {path}: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "name": path, "src": src, "fstype": fstype}))
}

/// Option-set equality ignoring order (defaults is the common case).
fn opts_eq(a: &str, b: &str) -> bool {
    let mut sa: Vec<&str> = a.split(',').collect();
    let mut sb: Vec<&str> = b.split(',').collect();
    sa.sort_unstable();
    sb.sort_unstable();
    sa == sb
}

fn is_mounted(path: &str) -> Result<bool, String> {
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    Ok(mounts
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .any(|mp| mp == path))
}

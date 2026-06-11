//! `community.general.lvg` (SEMANTICS §6): VG exists with exactly the
//! given PVs. ⚠ resolved 2026-06-11: create when absent (pvcreate each PV,
//! then vgcreate); when present with a *superset* requested, extend
//! (vgextend the new PVs); a requested *subset* is a no-op (Ansible does
//! not auto-reduce without force) — the drive-add workflow is "append a
//! disk, re-run". Reduction is out of the workload's surface.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let vg = str_param(obj, "vg").ok_or("lvg: vg required")?;
    let pvs: Vec<String> = match obj.get("pvs") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or("lvg: pvs entries must be strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(Value::String(s)) => s.split(',').map(|p| p.trim().to_string()).collect(),
        _ => return Err("lvg: pvs required (list of device paths)".into()),
    };
    let want: BTreeSet<String> = pvs.iter().cloned().collect();

    let existing = current_pvs(vg)?;
    let mut changed = false;

    match existing {
        None => {
            changed = true;
            if !ctx.check_mode {
                for pv in &pvs {
                    // pvcreate is idempotent-ish; -f -y to claim cleanly.
                    run_cmd("pvcreate", &["-f", "-y", pv])?;
                }
                let mut args = vec![vg];
                for pv in &pvs {
                    args.push(pv);
                }
                run_cmd("vgcreate", &args)?;
            }
        }
        Some(have) => {
            let new_pvs: Vec<&String> = pvs.iter().filter(|p| !have.contains(*p)).collect();
            if !new_pvs.is_empty() {
                // Requested PVs the VG does not have yet → extend.
                changed = true;
                if !ctx.check_mode {
                    for pv in &new_pvs {
                        run_cmd("pvcreate", &["-f", "-y", pv])?;
                    }
                    let mut args = vec![vg];
                    for pv in &new_pvs {
                        args.push(pv);
                    }
                    run_cmd("vgextend", &args)?;
                }
            }
            // A requested subset (have ⊃ want) is intentionally a no-op.
            let _ = &want;
        }
    }

    Ok(json!({"changed": changed, "failed": false, "vg": vg}))
}

/// PVs currently in the VG, or None when the VG does not exist.
fn current_pvs(vg: &str) -> Result<Option<BTreeSet<String>>, String> {
    let out = std::process::Command::new("vgs")
        .args([
            "--noheadings",
            "-o",
            "pv_name",
            "--reportformat",
            "json",
            vg,
        ])
        .output()
        .map_err(|e| format!("exec vgs: {e}"))?;
    if !out.status.success() {
        // vgs exits non-zero when the VG is absent.
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let parsed: Value = serde_json::from_str(&text).map_err(|e| format!("vgs json: {e}"))?;
    // vgs nests the pv_name rows under the "vg" array (verified against
    // lvm2 reportformat json 2026-06-11), not a "pv" array.
    let mut set = BTreeSet::new();
    if let Some(report) = parsed["report"].as_array() {
        for r in report {
            for key in ["vg", "pv"] {
                if let Some(rows) = r[key].as_array() {
                    for row in rows {
                        if let Some(name) = row["pv_name"].as_str() {
                            set.insert(name.trim().to_string());
                        }
                    }
                }
            }
        }
    }
    Ok(Some(set))
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

//! `apt_repository` (SEMANTICS §6): exact sources line in
//! /etc/apt/sources.list.d/<filename>.list. Changed iff the file's
//! content changed; update_cache refreshes lists after a change (cache
//! refresh itself never reports changed — same pin as apt).

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let repo = str_param(obj, "repo").ok_or("apt_repository: repo required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "apt_repository: state {state:?} outside the closed surface"
        ));
    }
    let filename = str_param(obj, "filename").ok_or(
        "apt_repository: filename required (the workload always sets it; \
         Ansible's auto-naming is out of surface)",
    )?;
    let update_cache = bool_param(obj, "update_cache", true);

    let path = PathBuf::from(format!("/etc/apt/sources.list.d/{filename}.list"));
    let want = format!("{repo}\n");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    // Ansible appends the line if missing rather than overwriting other
    // lines; the workload's files are single-line and ruxel-owned, so
    // exact-content compare is equivalent — and stricter.
    let changed = current != want;

    if changed && !ctx.check_mode {
        std::fs::write(&path, &want).map_err(|e| e.to_string())?;
        if update_cache {
            let out = std::process::Command::new("apt-get")
                .arg("update")
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output()
                .map_err(|e| format!("apt-get update: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "apt-get update after repo change failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
    }

    Ok(json!({
        "changed": changed,
        "failed": false,
        "repo": repo,
        "state": state,
    }))
}

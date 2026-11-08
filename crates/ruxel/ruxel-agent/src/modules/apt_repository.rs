#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, bool_param, params_object, str_param, write_atomic};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_in(params, ctx, Path::new("/etc/apt/sources.list.d"))
}

fn run_in(params: &Value, ctx: &ExecContext, directory: &Path) -> Result<Value, String> {
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

    validate_filename(filename)?;
    let path = directory.join(format!("{filename}.list"));
    let want = format!("{repo}\n");
    let current = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "apt_repository: cannot read {}: {error}",
                path.display()
            ));
        }
    };
    let changed = current != want;

    if changed && !ctx.check_mode {
        write_atomic(&path, want.as_bytes())?;
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

    let mut result = json!({
        "changed": changed,
        "failed": false,
        "repo": repo,
        "state": state,
    });
    if changed && ctx.diff_mode && !ctx.no_log {
        result["diff"] = json!(super::unified_diff(&current, &want));
    }
    Ok(result)
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() || filename.contains('/') || filename.contains("..") {
        Err(format!("apt_repository: invalid filename {filename:?}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;

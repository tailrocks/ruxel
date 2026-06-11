//! `get_url` (SEMANTICS §6): url + dest only. Pinned semantics: with no
//! checksum and force unset, an existing dest short-circuits to unchanged;
//! otherwise download and report changed. Fetching shells out to curl
//! (falling back to wget) — the workload installs curl before its only
//! get_url uses, and a static-musl HTTP+TLS stack inside the agent buys
//! nothing but binary size until a playbook needs it.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let url = str_param(obj, "url").ok_or("get_url: url required")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!(
            "get_url: only http(s) urls are in the closed surface: {url}"
        ));
    }
    let dest = str_param(obj, "dest").ok_or("get_url: dest required")?;
    let p = Path::new(dest);

    if p.exists() {
        return Ok(json!({
            "changed": false,
            "failed": false,
            "dest": dest,
            "url": url,
            "msg": "file already exists",
        }));
    }

    if ctx.check_mode {
        return Ok(json!({
            "changed": true,
            "failed": false,
            "dest": dest,
            "url": url,
        }));
    }

    if let Some(parent) = p.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let tmp = p.with_file_name(format!(
        ".{}.ruxel-dl",
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "get_url".into())
    ));
    let fetched = fetch(url, &tmp)?;
    if !fetched {
        return Err(format!(
            "get_url: neither curl nor wget available to fetch {url}"
        ));
    }
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())?;

    Ok(json!({
        "changed": true,
        "failed": false,
        "dest": dest,
        "url": url,
    }))
}

fn fetch(url: &str, dest: &Path) -> Result<bool, String> {
    let curl = std::process::Command::new("curl")
        .arg("-fsSL")
        .arg("-o")
        .arg(dest)
        .arg("--")
        .arg(url)
        .output();
    if let Ok(out) = curl {
        if out.status.success() {
            return Ok(true);
        }
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let wget = std::process::Command::new("wget")
        .arg("-q")
        .arg("-O")
        .arg(dest)
        .arg("--")
        .arg(url)
        .output();
    match wget {
        Ok(out) if out.status.success() => Ok(true),
        Ok(out) => Err(format!(
            "wget failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(_) => Ok(false),
    }
}

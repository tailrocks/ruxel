#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

mod access;
mod client;
mod staging;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_with_fetch(params, ctx, |url, dest| fetch(url, dest, ctx))
}

fn run_with_fetch(
    params: &Value,
    ctx: &ExecContext,
    fetch: impl FnOnce(&str, &std::fs::File) -> Result<bool, String>,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let url = str_param(obj, "url").ok_or("get_url: url required")?;
    validate_url(url)?;
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

    let tmp = staging::Download::new(p)?;
    let fetched = fetch(url, &tmp.file)?;
    if !fetched {
        return Err(format!(
            "get_url: neither curl nor wget available to fetch {url}"
        ));
    }
    tmp.publish(p)?;

    Ok(json!({
        "changed": true,
        "failed": false,
        "dest": dest,
        "url": url,
    }))
}

fn validate_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(format!(
            "get_url: only http(s) urls are in the closed surface: {url}"
        ))
    }
}

fn curl_args(url: &str) -> [&str; 4] {
    ["--disable", "-fsSL", "--", url]
}

fn fetch(url: &str, dest: &std::fs::File, ctx: &ExecContext) -> Result<bool, String> {
    let curl = client::resolve("curl")?;
    let wget = client::resolve("wget")?;
    fetch_with_programs(url, dest, curl.as_deref(), wget.as_deref(), ctx)
}

fn fetch_with_programs(
    url: &str,
    dest: &std::fs::File,
    curl: Option<&Path>,
    wget: Option<&Path>,
    ctx: &ExecContext,
) -> Result<bool, String> {
    if let Some(curl) = curl {
        let out = client::command(curl, ctx)
            .args(curl_args(url))
            .stdout(dest.try_clone().map_err(|e| e.to_string())?)
            .output()
            .map_err(|e| format!("curl could not start: {e}"))?;
        if out.status.success() {
            return Ok(true);
        }
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let Some(wget) = wget else {
        return Ok(false);
    };
    let wget = client::command(wget, ctx)
        .arg("--no-config")
        .arg("-q")
        .arg("-O")
        .arg("-")
        .arg("--")
        .arg(url)
        .stdout(dest.try_clone().map_err(|e| e.to_string())?)
        .output();
    match wget {
        Ok(out) if out.status.success() => Ok(true),
        Ok(out) => Err(format!(
            "wget failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("wget could not start: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::ExecContext;
    use serde_json::json;

    #[test]
    fn validates_scheme_and_places_separator_before_url() {
        assert!(super::validate_url("https://example.test/a").is_ok());
        assert!(super::validate_url("file:///etc/passwd").is_err());
        let args = super::curl_args("https://example.test/a");
        assert_eq!(args[0], "--disable");
        assert_eq!(args[2], "--");
        assert_eq!(args[3], "https://example.test/a");
    }

    #[test]
    fn existing_destination_short_circuits_without_fetch() {
        let path = std::env::temp_dir().join(format!("ruxel-get-url-{}", std::process::id()));
        std::fs::write(&path, "existing").unwrap();
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = super::run(
            &json!({"url":"https://example.invalid/file", "dest":path}),
            &ctx,
        )
        .unwrap();
        assert_eq!(result["changed"], false);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
#[path = "get_url/tests.rs"]
mod download_tests;

#[cfg(all(test, target_vendor = "apple"))]
#[path = "get_url/tests/acl.rs"]
mod acl_tests;

#[cfg(test)]
#[path = "get_url/tests/parents.rs"]
mod parent_tests;

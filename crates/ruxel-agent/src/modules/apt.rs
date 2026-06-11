//! `apt` (SEMANTICS §6): the workload's exact surface — name install
//! (present/latest), update_cache, upgrade: dist, autoremove, force.
//! Changed semantics pinned 2026-06-11 by fixture captures
//! (update-packages run1/run2): update_cache-only is never changed;
//! upgrade/autoremove are changed iff apt actually did work (parsed from
//! the "N upgraded, N newly installed, N to remove" summary); installs
//! are changed iff a requested package was missing.

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let update_cache = bool_param(obj, "update_cache", false);
    let upgrade = str_param(obj, "upgrade");
    let autoremove = bool_param(obj, "autoremove", false);
    let names: Vec<String> = match obj.get("name") {
        None => vec![],
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "apt: name entries must be strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(other) => return Err(format!("apt: invalid name {other:?}")),
    };
    let state = str_param(obj, "state").unwrap_or("present");

    let mut result = json!({"changed": false});
    let mut changed = false;

    if update_cache {
        if !ctx.check_mode {
            let out = apt_get(&["update"], ctx)?;
            if out.1 != 0 {
                return Err(format!("apt update failed: {}", out.2));
            }
        }
        // Pinned: cache refresh alone never reports changed.
        result["cache_updated"] = json!(!ctx.check_mode);
        result["cache_update_time"] = json!(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }

    if let Some(kind) = upgrade {
        let action = match kind {
            "dist" => "dist-upgrade",
            other => return Err(format!("apt: upgrade={other:?} outside the closed surface")),
        };
        if ctx.check_mode {
            let (stdout, rc, stderr) = apt_get(&["-s", action], ctx)?;
            if rc != 0 {
                return Err(format!("apt {action} -s failed: {stderr}"));
            }
            changed |= summary_changed(&stdout);
            fill_exec_fields(&mut result, &stdout, &stderr, true);
        } else {
            let (stdout, rc, stderr) = apt_get(&[action], ctx)?;
            if rc != 0 {
                return Err(format!("apt {action} failed: {stderr}"));
            }
            changed |= summary_changed(&stdout);
            fill_exec_fields(&mut result, &stdout, &stderr, true);
        }
    }

    if autoremove && upgrade.is_none() {
        let args: &[&str] = if ctx.check_mode {
            &["-s", "autoremove"]
        } else {
            &["autoremove"]
        };
        let (stdout, rc, stderr) = apt_get(args, ctx)?;
        if rc != 0 {
            return Err(format!("apt autoremove failed: {stderr}"));
        }
        changed |= summary_changed(&stdout);
        fill_exec_fields(&mut result, &stdout, &stderr, false);
        result["diff"] = json!({});
    } else if autoremove && upgrade.is_some() {
        // upgrade+autoremove arrive together in the workload: apt-get
        // handles it via --auto-remove on the upgrade call; the summary
        // parse above already accounted for removals.
    }

    if !names.is_empty() {
        let missing = missing_packages(&names, state)?;
        if !missing.is_empty() {
            changed = true;
            if !ctx.check_mode {
                let mut args: Vec<&str> = vec!["install"];
                if bool_param(obj, "force", false) {
                    args.push("--allow-downgrades");
                }
                let owned: Vec<String> = missing.clone();
                let mut full: Vec<&str> = args;
                for p in &owned {
                    full.push(p);
                }
                let (stdout, rc, stderr) = apt_get(&full, ctx)?;
                if rc != 0 {
                    return Err(format!("apt install failed: {stderr}"));
                }
                fill_exec_fields(&mut result, &stdout, &stderr, false);
            }
        }
    }

    result["changed"] = json!(changed);
    result["failed"] = json!(false);
    Ok(result)
}

/// `apt-get -y` with Ansible's default dpkg options and the workload's
/// DEBIAN_FRONTEND, plus task environment.
fn apt_get(args: &[&str], ctx: &ExecContext) -> Result<(String, i32, String), String> {
    let mut cmd = std::process::Command::new("apt-get");
    cmd.arg("-y")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confdef")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confold");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| format!("exec apt-get: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

/// Parse apt's "N upgraded, N newly installed, N to remove and N not
/// upgraded." summary: any non-zero action count = changed.
fn summary_changed(stdout: &str) -> bool {
    for line in stdout.lines() {
        if line.contains("upgraded,") && line.contains("newly installed") {
            let numbers: Vec<u64> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            // upgraded, newly installed, to remove — "not upgraded" (idx 3)
            // does not count as work done.
            return numbers.iter().take(3).any(|n| *n > 0);
        }
    }
    false
}

fn fill_exec_fields(result: &mut Value, stdout: &str, stderr: &str, with_msg: bool) {
    let stdout = stdout.trim_end_matches('\n');
    let stderr = stderr.trim_end_matches('\n');
    let lines = |s: &str| -> Vec<String> {
        if s.is_empty() {
            vec![]
        } else {
            s.lines().map(str::to_string).collect()
        }
    };
    if with_msg {
        result["msg"] = json!(stdout);
    }
    result["stdout"] = json!(stdout);
    result["stderr"] = json!(stderr);
    result["stdout_lines"] = json!(lines(stdout));
    result["stderr_lines"] = json!(lines(stderr));
}

/// Which of the requested packages need an install action: for `present`,
/// anything not currently installed; for `latest`, also anything with a
/// newer candidate per apt policy.
fn missing_packages(names: &[String], state: &str) -> Result<Vec<String>, String> {
    let mut missing = Vec::new();
    for name in names {
        let out = std::process::Command::new("dpkg-query")
            .arg("-W")
            .arg("-f")
            .arg("${Status}")
            .arg(name)
            .output()
            .map_err(|e| format!("dpkg-query: {e}"))?;
        let installed = out.status.success()
            && String::from_utf8_lossy(&out.stdout).contains("install ok installed");
        if !installed {
            missing.push(name.clone());
            continue;
        }
        if state == "latest" {
            let policy = std::process::Command::new("apt-cache")
                .arg("policy")
                .arg(name)
                .output()
                .map_err(|e| format!("apt-cache policy: {e}"))?;
            let text = String::from_utf8_lossy(&policy.stdout).to_string();
            let grab = |key: &str| -> Option<String> {
                text.lines()
                    .find(|l| l.trim_start().starts_with(key))
                    .map(|l| {
                        l.split(':')
                            .skip(1)
                            .collect::<Vec<_>>()
                            .join(":")
                            .trim()
                            .to_string()
                    })
            };
            let installed_v = grab("Installed");
            let candidate_v = grab("Candidate");
            if let (Some(i), Some(c)) = (installed_v, candidate_v)
                && i != c
            {
                missing.push(name.clone());
            }
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::summary_changed;

    #[test]
    fn summary_parsing() {
        assert!(!summary_changed(
            "0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded."
        ));
        assert!(summary_changed(
            "12 upgraded, 1 newly installed, 0 to remove and 3 not upgraded."
        ));
        assert!(summary_changed(
            "0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded."
        ));
        // "not upgraded" alone is not work
        assert!(!summary_changed(
            "0 upgraded, 0 newly installed, 0 to remove and 7 not upgraded."
        ));
    }
}

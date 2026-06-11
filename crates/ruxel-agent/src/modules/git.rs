//! `git` (SEMANTICS §6): repo/dest/version(branch)/update/force/
//! accept_hostkey. update=false → clone only if absent. Changed = fresh
//! clone or HEAD sha moved; `force` discards local modifications before
//! update. Network-truth class (ARCHITECTURE §6).

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let repo = str_param(obj, "repo").ok_or("git: repo required")?;
    let dest = str_param(obj, "dest").ok_or("git: dest required")?;
    let version = str_param(obj, "version");
    let update = bool_param(obj, "update", true);
    let force = bool_param(obj, "force", false);
    let accept_hostkey = bool_param(obj, "accept_hostkey", false);

    let dest_git = Path::new(dest).join(".git");
    let exists = dest_git.is_dir();

    let mut env_ssh = String::new();
    if accept_hostkey {
        env_ssh = "ssh -o StrictHostKeyChecking=accept-new".to_string();
    }
    let git = |args: &[&str], cwd: Option<&str>| -> Result<(String, bool), String> {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args);
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        if !env_ssh.is_empty() {
            cmd.env("GIT_SSH_COMMAND", &env_ssh);
        }
        for (k, v) in &ctx.environment {
            cmd.env(k, v);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("exec git {}: {e}", args.join(" ")))?;
        Ok((
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            out.status.success(),
        ))
    };

    if !exists {
        if ctx.check_mode {
            return Ok(json!({"changed": true, "failed": false, "before": null, "after": null}));
        }
        let mut args: Vec<&str> = vec!["clone"];
        if let Some(v) = version {
            args.push("--branch");
            args.push(v);
        }
        args.push(repo);
        args.push(dest);
        let (_, ok) = git(&args, None)?;
        if !ok {
            return Err(format!("git clone {repo} failed"));
        }
        let (after, _) = git(&["rev-parse", "HEAD"], Some(dest))?;
        return Ok(json!({"changed": true, "failed": false, "before": null, "after": after}));
    }

    let (before, _) = git(&["rev-parse", "HEAD"], Some(dest))?;
    if !update {
        return Ok(json!({"changed": false, "failed": false, "before": before, "after": before}));
    }

    if ctx.check_mode {
        // Compare remote HEAD for the branch without touching the tree.
        let branch = version.unwrap_or("HEAD");
        let (ls, ok) = git(&["ls-remote", repo, branch], Some(dest))?;
        let remote = ls.split_whitespace().next().unwrap_or("").to_string();
        let changed = ok && !remote.is_empty() && remote != before;
        return Ok(json!({"changed": changed, "failed": false, "before": before, "after": remote}));
    }

    if force {
        let (_, ok) = git(&["reset", "--hard"], Some(dest))?;
        if !ok {
            return Err("git reset --hard failed".into());
        }
    }
    let (_, ok) = git(&["fetch", "origin"], Some(dest))?;
    if !ok {
        return Err("git fetch failed".into());
    }
    let target = match version {
        Some(v) => format!("origin/{v}"),
        None => "FETCH_HEAD".to_string(),
    };
    let (_, ok) = git(&["checkout", version.unwrap_or("HEAD")], Some(dest))?;
    if !ok {
        return Err("git checkout failed".into());
    }
    let (_, ok) = git(&["reset", "--hard", &target], Some(dest))?;
    if !ok {
        return Err(format!("git reset --hard {target} failed"));
    }
    let (after, _) = git(&["rev-parse", "HEAD"], Some(dest))?;
    Ok(json!({
        "changed": before != after,
        "failed": false,
        "before": before,
        "after": after,
    }))
}

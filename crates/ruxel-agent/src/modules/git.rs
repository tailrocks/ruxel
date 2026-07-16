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

    // argv flag-smuggling defense: positional values must not look like
    // flags (`--` placement is subcommand-sensitive in git — `checkout --
    // x` means path x — so validation, not separators, is the guard).
    validate_positionals(&[
        ("repo", Some(repo)),
        ("dest", Some(dest)),
        ("version", version),
    ])?;

    let dest_git = Path::new(dest).join(".git");
    let exists = dest_git.is_dir();
    let action = repo_action(exists, update, ctx.check_mode);

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

    if action == "clone" {
        if ctx.check_mode {
            return Ok(json!({"changed": true, "failed": false, "before": null, "after": null}));
        }
        let mut args: Vec<&str> = vec!["clone"];
        if let Some(v) = clone_branch(version) {
            args.push("--branch");
            args.push(v);
        }
        args.push("--");
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
    if action == "unchanged" {
        return Ok(json!({"changed": false, "failed": false, "before": before, "after": before}));
    }

    if action == "compare" {
        // Compare remote HEAD for the branch without touching the tree.
        let branch = version.unwrap_or("HEAD");
        let (ls, ok) = git(&["ls-remote", "--", repo, branch], Some(dest))?;
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

fn clone_branch(version: Option<&str>) -> Option<&str> {
    version.filter(|version| *version != "HEAD")
}

fn validate_positionals(values: &[(&str, Option<&str>)]) -> Result<(), String> {
    for (label, value) in values {
        if value.is_some_and(|value| value.starts_with('-')) {
            return Err(format!(
                "git: refusing {label} that looks like a flag: {value:?}"
            ));
        }
    }
    Ok(())
}

fn repo_action(exists: bool, update: bool, check_mode: bool) -> &'static str {
    match (exists, update, check_mode) {
        (false, _, _) => "clone",
        (true, false, _) => "unchanged",
        (true, true, true) => "compare",
        (true, true, false) => "update",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn positional_flags_are_rejected() {
        assert!(super::validate_positionals(&[("repo", Some("https://example.test/x"))]).is_ok());
        for field in ["repo", "dest", "version"] {
            assert!(super::validate_positionals(&[(field, Some("--evil"))]).is_err());
        }
    }

    #[test]
    fn chooses_clone_update_compare_or_noop() {
        assert_eq!(super::repo_action(false, false, false), "clone");
        assert_eq!(super::repo_action(true, false, false), "unchanged");
        assert_eq!(super::repo_action(true, true, true), "compare");
        assert_eq!(super::repo_action(true, true, false), "update");
        assert_eq!(super::clone_branch(Some("HEAD")), None);
        assert_eq!(super::clone_branch(Some("main")), Some("main"));
    }
}

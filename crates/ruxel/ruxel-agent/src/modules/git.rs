#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

#[path = "git/check.rs"]
mod check;
#[path = "git/checkout.rs"]
mod checkout;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let repo = str_param(obj, "repo").ok_or("git: repo required")?;
    let dest = str_param(obj, "dest").ok_or("git: dest required")?;
    let version = str_param(obj, "version");
    let update = bool_param(obj, "update", true);
    let force = bool_param(obj, "force", false);
    let accept_hostkey = bool_param(obj, "accept_hostkey", false);

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
    let head = || -> Result<String, String> {
        let (revision, ok) = git(&["rev-parse", "HEAD"], Some(dest))?;
        if !ok {
            return Err("git rev-parse HEAD failed".into());
        }
        Ok(revision)
    };

    if action == "clone" {
        if ctx.check_mode {
            return Ok(json!({"changed": true, "failed": false, "before": null, "after": null}));
        }
        let mut args: Vec<&str> = vec!["clone"];
        args.push("--");
        args.push(repo);
        args.push(dest);
        let (_, ok) = git(&args, None)?;
        if !ok {
            return Err(format!("git clone {repo} failed"));
        }
        if let Some(v) = clone_branch(version) {
            checkout::requested(v, dest, &git)?;
        }
        let after = head()?;
        return Ok(json!({"changed": true, "failed": false, "before": null, "after": after}));
    }

    let before = head()?;
    if action == "unchanged" {
        return Ok(json!({"changed": false, "failed": false, "before": before, "after": before}));
    }

    let (status, ok) = git(&["status", "--porcelain"], Some(dest))?;
    if !ok {
        return Err("git status failed".into());
    }
    let dirty = !status.is_empty();
    let tracked_dirty = status.lines().any(|line| !line.starts_with("?? "));
    if dirty && !force {
        return Err("git: local changes would be overwritten; set force to discard them".into());
    }

    if action == "compare" {
        let branch = version.unwrap_or("HEAD");
        let (ls, ok) = git(&["ls-remote", "--", repo, branch], Some(dest))?;
        let mut remote = ls.split_whitespace().next().unwrap_or("").to_string();
        if let Some(tag_ref) = ls
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            && tag_ref.starts_with("refs/tags/")
        {
            let peeled = format!("{tag_ref}^{{}}");
            let (tag, tag_ok) = git(&["ls-remote", "--", repo, &peeled], Some(dest))?;
            if tag_ok && !tag.is_empty() {
                remote = tag.split_whitespace().next().unwrap_or("").to_string();
            }
        }
        if ok && remote.is_empty() {
            let is_oid = is_object_id(branch);
            let hash_len = if is_oid {
                let (format, found) = git(&["rev-parse", "--show-object-format"], Some(dest))?;
                if !found {
                    return Err("git: cannot determine object format".into());
                }
                match format.as_str() {
                    "sha1" => 40,
                    "sha256" => 64,
                    _ => return Err("git: unsupported object format".into()),
                }
            } else {
                0
            };
            if is_oid && branch.len() < hash_len {
                remote = check::resolve(repo, dest, branch, &git)?;
            } else {
                let (resolved, found) = git(
                    &["rev-parse", "--verify", &format!("{branch}^{{commit}}")],
                    Some(dest),
                )?;
                if found {
                    remote = resolved;
                } else if is_oid {
                    let (refs, listed) = git(&["ls-remote", "--", repo], Some(dest))?;
                    if !listed {
                        return Err("git ls-remote failed".into());
                    }
                    remote = match advertised_object(&refs, branch)? {
                        Some(oid) => oid,
                        None => check::resolve(repo, dest, branch, &git)?,
                    };
                } else {
                    return Err(format!("git: cannot resolve version {branch}"));
                }
            }
        }
        if !ok {
            return Err("git ls-remote failed".into());
        }
        let changed = tracked_dirty || remote != before;
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
    match version {
        Some(v) => checkout::requested(v, dest, &git)?,
        None => checkout::resolved("FETCH_HEAD", dest, &git)?,
    }
    let after = head()?;
    let (after_status, ok) = git(&["status", "--porcelain"], Some(dest))?;
    if !ok {
        return Err("git status after update failed".into());
    }
    Ok(json!({
        "changed": status != after_status || before != after,
        "failed": false,
        "before": before,
        "after": after,
    }))
}

fn is_object_id(version: &str) -> bool {
    (4..=64).contains(&version.len()) && version.bytes().all(|b| b.is_ascii_hexdigit())
}

fn advertised_object(refs: &str, version: &str) -> Result<Option<String>, String> {
    let prefix = version.to_ascii_lowercase();
    let mut matches = std::collections::BTreeSet::new();
    for line in refs.lines() {
        let Some((oid, name)) = line.split_once('\t') else {
            continue;
        };
        if name != "HEAD" && !name.starts_with("refs/heads/") {
            continue;
        }
        if oid.starts_with(&prefix) {
            matches.insert(oid);
        }
    }
    if matches.len() > 1 {
        return Err(format!("git: ambiguous advertised object {version}"));
    }
    Ok(matches.into_iter().next().map(str::to_owned))
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
    use serde_json::json;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn update_preserves_dirty_checkout_unless_forced() {
        let root = std::env::temp_dir().join(format!("ruxel-git-test-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source");
        let dest = root.join("dest");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Test"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(source.join("file"), "original\n").unwrap();
        git(&source, &["add", "file"]);
        git(&source, &["commit", "-m", "initial"]);
        git(
            &root,
            &["clone", source.to_str().unwrap(), dest.to_str().unwrap()],
        );
        std::fs::write(dest.join("file"), "local edit\n").unwrap();
        let ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let params = json!({"repo": source, "dest": dest});
        assert!(
            super::run(&params, &ctx)
                .unwrap_err()
                .contains("local changes")
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("file")).unwrap(),
            "local edit\n"
        );
        let mut forced = params;
        forced["force"] = json!(true);
        assert_eq!(super::run(&forced, &ctx).unwrap()["changed"], true);
        assert_eq!(
            std::fs::read_to_string(dest.join("file")).unwrap(),
            "original\n"
        );
        assert_eq!(super::run(&forced, &ctx).unwrap()["changed"], false);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clone_and_update_tag_and_commit_versions() {
        let root = std::env::temp_dir().join(format!("ruxel-git-versions-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source");
        let dest = root.join("dest");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Test"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(source.join("file"), "first").unwrap();
        git(&source, &["add", "file"]);
        git(&source, &["commit", "-m", "first"]);
        git(&source, &["tag", "-a", "v1", "-m", "version one"]);
        let first = Command::new("git")
            .current_dir(&source)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let first = String::from_utf8(first.stdout).unwrap().trim().to_owned();
        std::fs::write(source.join("file"), "second").unwrap();
        git(&source, &["commit", "-am", "second"]);
        let second = Command::new("git")
            .current_dir(&source)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let second = String::from_utf8(second.stdout).unwrap().trim().to_owned();
        let ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let mut params = json!({"repo": source, "dest": dest, "version": first});
        assert_eq!(super::run(&params, &ctx).unwrap()["after"], first);
        params["version"] = json!(second);
        assert_eq!(super::run(&params, &ctx).unwrap()["after"], second);
        params["version"] = json!("v1");
        assert_eq!(super::run(&params, &ctx).unwrap()["after"], first);
        let check = super::ExecContext {
            check_mode: true,
            ..ctx
        };
        assert_eq!(super::run(&params, &check).unwrap()["changed"], false);
        std::fs::remove_dir_all(root).unwrap();
    }

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

#[cfg(test)]
#[path = "git/tests/pinned.rs"]
mod pinned_tests;

#[cfg(test)]
#[path = "git/tests/ambiguity.rs"]
mod ambiguity_tests;

#[cfg(test)]
#[path = "git/tests/head.rs"]
mod head_tests;

#[cfg(test)]
#[path = "git/tests/branches.rs"]
mod branch_tests;

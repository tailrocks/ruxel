#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

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
        let completed = (|| {
            if let Some(v) = clone_branch(version) {
                checkout::requested(v, dest, force, &git)?;
            }
            head()
        })();
        return Ok(match completed {
            Ok(after) => json!({"changed": true, "failed": false, "before": null, "after": after}),
            Err(error) => json!({"changed": true, "failed": true, "msg": error}),
        });
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
        let short_oid = if is_object_id(branch) {
            let (format, found) = git(&["rev-parse", "--show-object-format"], Some(dest))?;
            if !found {
                return Err("git: cannot determine object format".into());
            }
            let hash_len = match format.as_str() {
                "sha1" => 40,
                "sha256" => 64,
                _ => return Err("git: unsupported object format".into()),
            };
            branch.len() < hash_len
        } else {
            false
        };
        let remote_source = "origin";
        let (mut ls, ok) = if short_oid {
            let branch_ref = format!("refs/heads/{branch}");
            let tag_ref = format!("refs/tags/{branch}");
            git(
                &["ls-remote", "--", remote_source, &branch_ref, &tag_ref],
                Some(dest),
            )?
        } else {
            git(&["ls-remote", "--", remote_source, branch], Some(dest))?
        };
        if !ok {
            return Err("git ls-remote failed".into());
        }
        if short_oid {
            let branch_ref = format!("refs/heads/{branch}");
            let tag_ref = format!("refs/tags/{branch}");
            ls = [branch_ref, tag_ref]
                .iter()
                .filter_map(|wanted| {
                    ls.lines().find(|line| {
                        line.split_once('\t')
                            .is_some_and(|(_, name)| name == wanted)
                    })
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        let mut remote = ls.split_whitespace().next().unwrap_or("").to_string();
        if let Some(tag_ref) = ls
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            && tag_ref.starts_with("refs/tags/")
        {
            let peeled = format!("{tag_ref}^{{}}");
            let (tag, tag_ok) = git(&["ls-remote", "--", remote_source, &peeled], Some(dest))?;
            if tag_ok && !tag.is_empty() {
                remote = tag.split_whitespace().next().unwrap_or("").to_string();
            }
        }
        if ok && remote.is_empty() {
            let is_oid = is_object_id(branch);
            {
                let (resolved, found) = git(
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{branch}^{{commit}}"),
                    ],
                    Some(dest),
                )?;
                if found {
                    remote = resolved;
                } else if short_oid {
                    return Err("git: abbreviated object IDs not available locally cannot be resolved in check mode; use a full object ID or ref".into());
                } else if is_oid {
                    let (refs, listed) = git(&["ls-remote", "--", remote_source], Some(dest))?;
                    if !listed {
                        return Err("git ls-remote failed".into());
                    }
                    remote = match advertised_object(&refs, branch)? {
                        Some(oid) => oid,
                        None => {
                            return Err(format!(
                                "git: cannot resolve version {branch} without fetching; use an advertised ref or a locally available commit"
                            ));
                        }
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
    let fetch_args: &[&str] = if version.is_none() {
        &["fetch", "origin", "HEAD"]
    } else {
        &["fetch", "origin"]
    };
    let (_, ok) = git(fetch_args, Some(dest))?;
    if !ok {
        return Err("git fetch failed".into());
    }
    match version {
        Some(v) if is_object_id(v) => {
            let branch_ref = format!("refs/heads/{v}");
            let tag_ref = format!("refs/tags/{v}");
            let (refs, ok) = git(
                &["ls-remote", "--", "origin", &branch_ref, &tag_ref],
                Some(dest),
            )?;
            if !ok {
                return Err("git ls-remote failed".into());
            }
            let has_ref = |wanted: &str| {
                refs.lines().any(|line| {
                    line.split_once('\t')
                        .is_some_and(|(_, name)| name == wanted)
                })
            };
            let remote_branch = format!("refs/remotes/origin/{v}");
            let requested = if has_ref(&branch_ref) {
                &remote_branch
            } else if has_ref(&tag_ref) {
                let (_, fetched) = git(&["fetch", "--no-tags", "origin", &tag_ref], Some(dest))?;
                if !fetched {
                    return Err(format!("git fetch {tag_ref} failed"));
                }
                "FETCH_HEAD"
            } else {
                v
            };
            checkout::resolved(requested, dest, force, &git)?;
        }
        Some(v) => checkout::requested(v, dest, force, &git)?,
        None => checkout::resolved("FETCH_HEAD", dest, force, &git)?,
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
        if name != "HEAD"
            && !name.starts_with("refs/heads/")
            && !(name.starts_with("refs/tags/") && name.ends_with("^{}"))
        {
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
#[path = "git/tests/basic.rs"]
mod tests;

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

#[cfg(test)]
#[path = "git/tests/collision.rs"]
mod collision_tests;

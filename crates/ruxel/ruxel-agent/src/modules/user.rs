#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};

use home::{claimed_home, ensure_home, home_gid, home_usable};
use shell::shell_allowed;

mod home;
mod shell;
use serde_json::{Value, json};

#[derive(Default)]
struct PasswdEntry {
    uid: u64,
    gid: u64,
    comment: String,
    home: String,
    shell: String,
}

pub(super) fn group(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("group: name required")?;
    if name.starts_with('-') {
        return Err("group: name must not start with a hyphen".into());
    }
    let state = str_param(obj, "state").unwrap_or("present");
    let gid = obj.get("gid").and_then(Value::as_u64);

    let entry = group_entry(name)?;
    let mut changed = false;

    match state {
        "present" => {
            if let Some((_, current_gid)) = entry {
                if let Some(want) = gid
                    && current_gid != want
                {
                    group_change_allowed(name, current_gid)?;
                    changed = true;
                    if !ctx.check_mode {
                        run_cmd("groupmod", &["-g", &want.to_string(), name])?;
                    }
                }
            } else {
                changed = true;
                if !ctx.check_mode {
                    let gid_s;
                    let mut args: Vec<&str> = Vec::new();
                    if let Some(want) = gid {
                        gid_s = want.to_string();
                        args.push("-g");
                        args.push(&gid_s);
                    }
                    args.push(name);
                    run_cmd("groupadd", &args)?;
                }
            }
        }
        "absent" => {
            if let Some((_, current_gid)) = entry {
                group_removal_allowed(name, gid, current_gid)?;
                changed = true;
                if !ctx.check_mode {
                    run_cmd("groupdel", &[name])?;
                }
            }
        }
        other => return Err(format!("group: state {other:?} outside the closed surface")),
    }
    Ok(json!({"changed": changed, "failed": false, "name": name}))
}

fn group_entry(name: &str) -> Result<Option<(String, u64)>, String> {
    let content = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    group_entry_from(&content, name)
}

fn group_entry_from(content: &str, name: &str) -> Result<Option<(String, u64)>, String> {
    for line in content.lines() {
        let mut f = line.split(':');
        if f.next() == Some(name) {
            let gid = f
                .nth(1)
                .and_then(|g| g.parse().ok())
                .ok_or("malformed group line")?;
            return Ok(Some((name.to_string(), gid)));
        }
    }
    Ok(None)
}

mod guards;
use guards::{
    group_change_allowed, group_removal_allowed, identity_change_allowed, planned_gid,
    primary_group_name, private_group_flag, private_group_missing, removal_allowed,
    removal_home_allowed, removal_primary_group_allowed,
};

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("user: name required")?;
    if name.starts_with('-') {
        return Err("user: name must not start with a hyphen".into());
    }
    let state = str_param(obj, "state").unwrap_or("present");

    let groups: Option<Vec<String>> = match obj.get("groups") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("user: groups entries must be strings")
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(other) => return Err(format!("user: invalid groups {other:?}")),
    };

    let entry = passwd_entry(name)?;
    let uid = obj.get("uid").and_then(Value::as_u64);

    if state == "absent" {
        // The deletion verifies the host entry, not the workflow value:
        // see `removal_allowed`. A recursive removal deletes the live
        // home whatever the workflow names, so the declared one must be
        // it: see `removal_home_allowed`. And the removal's primary
        // group must be the live one: see `removal_primary_group_allowed`.
        let home = str_param(obj, "home");
        let remove = bool_param(obj, "remove", false);
        if let Some(current) = &entry {
            removal_allowed(name, uid, current)?;
            removal_home_allowed(name, home, remove, &current.home)?;
            removal_primary_group_allowed(
                name,
                str_param(obj, "group"),
                primary_group_name(current.gid)?.as_deref(),
            )?;
        }
        let changed = entry.is_some();
        if changed && !ctx.check_mode {
            let mut args: Vec<&str> = Vec::new();
            if remove {
                args.push("-r");
            }
            args.push(name);
            run_cmd("userdel", &args)?;
        }
        return Ok(json!({"changed": changed, "failed": false, "name": name, "state": "absent"}));
    }

    let group = str_param(obj, "group");
    let append = bool_param(obj, "append", false);
    let home = str_param(obj, "home");
    let create_home = bool_param(obj, "create_home", true);
    let shell = str_param(obj, "shell");
    if let Some(sh) = shell {
        shell_allowed(sh)?;
    }
    let comment = str_param(obj, "comment");
    let system = bool_param(obj, "system", false);
    // The claimed default home is the step's managed path like a
    // declared one — `useradd -m` creates it, `usermod -d` points the
    // account at it — so a regular file or a symlink at it is refused
    // before the account is touched, declared or not.
    home_usable(&claimed_home(home, name))?;

    let mut changed = false;

    match entry {
        None => {
            changed = true;
            if !ctx.check_mode {
                let uid_s;
                let mut args: Vec<&str> = Vec::new();
                if system {
                    args.push("--system");
                }
                if let Some(u) = uid {
                    uid_s = u.to_string();
                    args.push("-u");
                    args.push(&uid_s);
                }
                if let Some(g) = group {
                    args.push("-g");
                    args.push(g);
                }
                if let Some(flag) = private_group_flag(group) {
                    args.push(flag);
                }
                let groups_s;
                if let Some(gs) = &groups {
                    groups_s = gs.join(",");
                    args.push("-G");
                    args.push(&groups_s);
                }
                let home_s = claimed_home(home, name);
                args.push("-d");
                args.push(&home_s);
                if create_home {
                    args.push("-m");
                } else {
                    args.push("-M");
                }
                if let Some(sh) = shell {
                    args.push("-s");
                    args.push(sh);
                }
                if let Some(c) = comment {
                    args.push("-c");
                    args.push(c);
                }
                args.push(name);
                run_cmd("useradd", &args)?;
                // `useradd -m` creates the claimed path only when it is
                // missing: a pre-existing directory keeps whatever
                // ownership it had, so the claimed home — declared or
                // the footprint's default — is reconciled after
                // creation with the account's final identity.
                if let Some(created) = passwd_entry(name)?
                    && (home.is_some() || create_home)
                {
                    ensure_home(
                        &claimed_home(home, name),
                        created.uid,
                        created.gid,
                        ctx.check_mode,
                    )?;
                }
            }
        }
        Some(current) => {
            // The step's footprint claims the account's private group
            // whenever it names no primary group, and the lowering's
            // group ordering advertises it as provided: an existing
            // account reconciles that claim — `useradd -U` only covers
            // the creation branch, and a dependent operation naming the
            // group would fail to resolve it.
            if private_group_missing(group, group_entry(name)?.is_some()) {
                changed = true;
                if !ctx.check_mode {
                    run_cmd("groupadd", &[name])?;
                }
            }
            let uid_s;
            let mut args: Vec<&str> = Vec::new();
            if let Some(u) = uid
                && u != current.uid
            {
                identity_change_allowed(name, &current)?;
                uid_s = u.to_string();
                args.push("-u");
                args.push(&uid_s);
            }
            if let Some(g) = group {
                match planned_gid(g, ctx.check_mode)? {
                    // The group is not on the host yet: an earlier
                    // same-stage `Group` step creates it and check mode
                    // runs no `groupadd`, so the plan reports the
                    // primary-group move without the new group's live
                    // gid. Apply resolves it after that step ran, and
                    // every live-entry guard applies there — a check
                    // mode mutation, so there is nothing to guard yet.
                    None => {
                        args.push("-g");
                        args.push(g);
                    }
                    Some(want_gid) => {
                        if u64::from(want_gid) != current.gid {
                            identity_change_allowed(name, &current)?;
                            args.push("-g");
                            args.push(g);
                        }
                    }
                }
            }
            let groups_s;
            if let Some(gs) = &groups {
                let current_groups = supplementary_groups(name)?;
                if groups_need_update(&current_groups, gs, append) {
                    groups_s = gs.join(",");
                    if append {
                        args.push("-a");
                    }
                    args.push("-G");
                    args.push(&groups_s);
                }
            }
            if let Some(h) = home
                && h != current.home
            {
                args.push("-d");
                args.push(h);
            }
            if let Some(sh) = shell
                && sh != current.shell
            {
                args.push("-s");
                args.push(sh);
            }
            if let Some(c) = comment
                && c != current.comment
            {
                args.push("-c");
                args.push(c);
            }
            if !args.is_empty() {
                changed = true;
                if !ctx.check_mode {
                    args.push(name);
                    run_cmd("usermod", &args)?;
                }
            }
            // The declared home is managed as a directory too: usermod -d
            // moves the passwd field only, so a missing directory is
            // reconciled here — an account whose home field already
            // matches still gets its directory.
            // `None` is check mode's answer for a group that is not on
            // the host yet: its gid decides the directory's owner, and
            // the primary-group change the plan already reports covers
            // the repair, so nothing is compared against a gid that
            // does not exist.
            if let Some(h) = home
                && let Some(gid) = home_gid(group, current.gid, ctx.check_mode)?
            {
                changed |= ensure_home(h, uid.unwrap_or(current.uid), gid, ctx.check_mode)?;
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "name": name, "state": "present"}))
}

fn passwd_entry(name: &str) -> Result<Option<PasswdEntry>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    passwd_entry_from(&passwd, name)
}

fn passwd_entry_from(passwd: &str, name: &str) -> Result<Option<PasswdEntry>, String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&name) && f.len() >= 7 {
            return Ok(Some(PasswdEntry {
                uid: f[2].parse().map_err(|_| "bad uid")?,
                gid: f[3].parse().map_err(|_| "bad gid")?,
                comment: f[4].to_string(),
                home: f[5].to_string(),
                shell: f[6].to_string(),
            }));
        }
    }
    Ok(None)
}

fn supplementary_groups(name: &str) -> Result<Vec<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(supplementary_groups_from(&groups, name))
}

fn supplementary_groups_from(groups: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[3].split(',').any(|m| m == name) {
            out.push(f[0].to_string());
        }
    }
    out
}

fn groups_need_update(current: &[String], want: &[String], append: bool) -> bool {
    let missing = want.iter().any(|group| !current.contains(group));
    (append && missing)
        || (!append
            && (current.len() != want.len() || want.iter().any(|group| !current.contains(group))))
}

#[cfg(test)]
mod tests;

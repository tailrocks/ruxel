#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};
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

/// Whether the agent may remove the account whose host entry is `entry`.
/// The deletion verifies the host entry rather than trusting the workflow
/// value: the superuser's account and a system account below UID 1000 are
/// the host's to keep whatever the workflow states, and a declared uid
/// must match the host's — `user = "daemon"; uid = 1000` must not remove
/// an account whose real UID is below 1000.
fn removal_allowed(name: &str, declared: Option<u64>, entry: &PasswdEntry) -> Result<(), String> {
    if entry.uid == 0 {
        return Err(format!(
            "user: refusing to remove {name}: the superuser's account (uid 0) is the host's to keep"
        ));
    }
    if entry.uid < 1000 {
        return Err(format!(
            "user: refusing to remove {name}: the host account is a system account (uid {}), which the host keeps",
            entry.uid
        ));
    }
    if let Some(want) = declared
        && want != entry.uid
    {
        return Err(format!(
            "user: refusing to remove {name}: the host uid {} is not the declared {want}",
            entry.uid
        ));
    }
    Ok(())
}

/// Whether the agent may remove the group of host gid `host_gid`. The
/// root group (GID 0) is the host's to keep whatever it is named, and a
/// declared gid must match the host's: the host entry, not the workflow
/// value, is what deletion acts on.
fn group_removal_allowed(name: &str, declared: Option<u64>, host_gid: u64) -> Result<(), String> {
    if host_gid == 0 {
        return Err(format!(
            "group: refusing to remove {name}: the root group (gid 0) is the host's to keep"
        ));
    }
    if let Some(want) = declared
        && want != host_gid
    {
        return Err(format!(
            "group: refusing to remove {name}: the host gid {host_gid} is not the declared {want}"
        ));
    }
    Ok(())
}

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
        // see `removal_allowed`.
        if let Some(current) = &entry {
            removal_allowed(name, uid, current)?;
        }
        let changed = entry.is_some();
        if changed && !ctx.check_mode {
            let mut args: Vec<&str> = Vec::new();
            if bool_param(obj, "remove", false) {
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
    let comment = str_param(obj, "comment");
    let system = bool_param(obj, "system", false);

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
                let groups_s;
                if let Some(gs) = &groups {
                    groups_s = gs.join(",");
                    args.push("-G");
                    args.push(&groups_s);
                }
                if let Some(h) = home {
                    args.push("-d");
                    args.push(h);
                }
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
            }
        }
        Some(current) => {
            let uid_s;
            let mut args: Vec<&str> = Vec::new();
            if let Some(u) = uid
                && u != current.uid
            {
                uid_s = u.to_string();
                args.push("-u");
                args.push(&uid_s);
            }
            if let Some(g) = group {
                let want_gid = super::resolve_gid(g)?;
                if u64::from(want_gid) != current.gid {
                    args.push("-g");
                    args.push(g);
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

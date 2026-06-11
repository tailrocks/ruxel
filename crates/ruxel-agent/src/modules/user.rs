//! `user` (SEMANTICS §6): passwd/group entries field-by-field via
//! useradd/usermod/userdel. The workload's surface: name, uid, group,
//! groups, append, home, create_home, shell, comment, system, state
//! (absent ×1), remove.

use super::misc::run_cmd;
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

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("user: name required")?;
    let state = str_param(obj, "state").unwrap_or("present");

    let entry = passwd_entry(name)?;

    if state == "absent" {
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

    let uid = obj.get("uid").and_then(Value::as_u64);
    let group = str_param(obj, "group");
    let groups: Option<Vec<String>> = match obj.get("groups") {
        None => None,
        Some(Value::String(s)) => Some(s.split(',').map(|g| g.trim().to_string()).collect()),
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        Some(other) => return Err(format!("user: invalid groups {other:?}")),
    };
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
                let missing: Vec<&String> =
                    gs.iter().filter(|g| !current_groups.contains(*g)).collect();
                let exact = !append
                    && (current_groups.len() != gs.len()
                        || gs.iter().any(|g| !current_groups.contains(g)));
                if (append && !missing.is_empty()) || exact {
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
    let mut out = Vec::new();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[3].split(',').any(|m| m == name) {
            out.push(f[0].to_string());
        }
    }
    Ok(out)
}

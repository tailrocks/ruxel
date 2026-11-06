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
            if entry.is_some() {
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

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
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
fn desired_groups(current: &[String], want: &[String], append: bool) -> Vec<String> {
    let mut desired = if append { current.to_vec() } else { Vec::new() };
    desired.extend(want.iter().cloned());
    desired.sort();
    desired.dedup();
    desired
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn rejects_option_like_group_names_before_running_utilities() {
        for check_mode in [true, false] {
            let ctx = ExecContext {
                check_mode,
                diff_mode: false,
                no_log: false,
                environment: Vec::new(),
                become_user: None,
            };
            for state in ["present", "absent"] {
                for name in ["--help", "-g"] {
                    let error = group(&json!({"name":name, "state":state}), &ctx)
                        .expect_err("option-like group name must be rejected");
                    assert!(error.contains("group") && error.contains("name"));
                }
            }
            if check_mode {
                assert!(group(&json!({"name":"ruxel-synthetic-group"}), &ctx).is_ok());
            }
        }
    }

    #[test]
    fn supplementary_group_reconciliation_is_set_based() {
        assert_eq!(
            desired_groups(&strings(&["a"]), &strings(&["b", "b"]), true),
            strings(&["a", "b"])
        );
        assert!(groups_need_update(&strings(&["a"]), &strings(&["b"]), true));
        assert!(!groups_need_update(
            &strings(&["a", "b"]),
            &strings(&["b", "a"]),
            false
        ));
        assert_eq!(
            desired_groups(&strings(&["a", "b"]), &strings(&["b"]), false),
            strings(&["b"])
        );
    }

    #[test]
    fn parses_passwd_and_group_content() {
        let entry = passwd_entry_from("alice:x:1001:1002:Alice:/home/alice:/bin/sh\n", "alice")
            .unwrap()
            .unwrap();
        assert_eq!(
            (entry.uid, entry.gid, entry.home.as_str()),
            (1001, 1002, "/home/alice")
        );
        assert_eq!(
            supplementary_groups_from("wheel:x:10:alice,bob\ndocker:x:20:bob\n", "alice"),
            strings(&["wheel"])
        );
    }
    #[test]
    fn parses_group_entry_from_canned_content() {
        assert_eq!(
            super::group_entry_from("wheel:x:10:a,b\n", "wheel").unwrap(),
            Some(("wheel".into(), 10))
        );
        assert_eq!(
            super::group_entry_from("wheel:x:10:a,b\n", "missing").unwrap(),
            None
        );
        assert!(super::group_entry_from("broken:x:nope:\n", "broken").is_err());
    }
}

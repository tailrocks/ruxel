//! Native module implementations (SEMANTICS §6): each takes its rendered
//! params (JSON) and produces the Ansible-shaped result dict the
//! controller registers and reports. Param closure is enforced at parse
//! time on the controller; the agent still rejects unknown params loudly
//! rather than ignoring them (defense in depth, same closed-surface rule).

mod apt;
mod apt_repository;
mod authorized_key;
mod blockinfile;
mod command;
mod copy;
mod file;
mod filesystem;
mod get_url;
mod git;
mod iptables;
mod lineinfile;
mod lvg;
mod lvol;
mod misc;
mod mount;
mod postgresql;
mod replace;
mod shell;
mod slurp;
mod stat;
mod sysctl;
mod systemd;
mod user;

use serde_json::{Map, Value, json};

pub struct ExecContext {
    pub check_mode: bool,
    /// Task `environment:` merged into the child process env.
    pub environment: Vec<(String, String)>,
    /// `become_user:` — run module subprocesses as this user (SEMANTICS §1).
    pub become_user: Option<String>,
}

pub struct Outcome {
    /// ok | changed | failed | skipped (status the controller reports).
    pub status: &'static str,
    pub changed: bool,
    pub result: Value,
}

impl Outcome {
    fn from_result(result: Value) -> Self {
        let failed = result["failed"].as_bool().unwrap_or(false);
        let changed = result["changed"].as_bool().unwrap_or(false);
        let skipped = result["skipped"].as_bool().unwrap_or(false);
        let status = if failed {
            "failed"
        } else if skipped {
            "skipped"
        } else if changed {
            "changed"
        } else {
            "ok"
        };
        Outcome {
            status,
            changed,
            result,
        }
    }
}

pub fn execute(module: &str, params: &Value, free_form: &str, ctx: &ExecContext) -> Outcome {
    let result = match module {
        "apt" => apt::run(params, ctx),
        "apt_repository" => apt_repository::run(params, ctx),
        "blockinfile" => blockinfile::run(params, ctx),
        "lineinfile" => lineinfile::run(params, ctx),
        "replace" => replace::run(params, ctx),
        "sysctl" | "ansible.posix.sysctl" => sysctl::run(params, ctx),
        "community.general.timezone" => misc::timezone(params, ctx),
        "group" => misc::group(params, ctx),
        "user" => user::run(params, ctx),
        "authorized_key" => authorized_key::run(params, ctx),
        "git" => git::run(params, ctx),
        "iptables" => iptables::run(params, ctx),
        "community.general.lvg" => lvg::run(params, ctx),
        "community.general.lvol" => lvol::run(params, ctx),
        "filesystem" => filesystem::run(params, ctx),
        "ansible.posix.mount" => mount::run(params, ctx),
        "template" => copy::run(params, ctx),
        "get_url" => get_url::run(params, ctx),
        "command" => command::run(params, free_form, ctx),
        "shell" => shell::run(params, free_form, ctx),
        "file" => file::run(params, ctx),
        "stat" => stat::run(params),
        "copy" => copy::run(params, ctx),
        "slurp" => slurp::run(params),
        "systemd" | "service" => systemd::run(params, ctx),
        "community.postgresql.postgresql_db" => postgresql::db(params, ctx),
        "community.postgresql.postgresql_user" => postgresql::user(params, ctx),
        "community.postgresql.postgresql_schema" => postgresql::schema(params, ctx),
        "community.postgresql.postgresql_privs" => postgresql::privs(params, ctx),
        other => Err(format!(
            "module {other:?} is not implemented in this agent build"
        )),
    };
    match result {
        Ok(value) => Outcome::from_result(value),
        Err(msg) => Outcome::from_result(json!({
            "failed": true,
            "changed": false,
            "msg": msg,
        })),
    }
}

// -- Shared helpers -----------------------------------------------------------

/// Build a std::process::Command, wrapped in `runuser -u <user> --` when the
/// task set `become_user` (SEMANTICS §1: the task runs as that uid/gid with
/// its environment). Used by command/shell and the postgresql modules
/// (peer auth as the postgres OS user).
pub(super) fn become_command(
    ctx: &ExecContext,
    program: &str,
    args: &[&str],
) -> std::process::Command {
    match &ctx.become_user {
        Some(user) => {
            let mut cmd = std::process::Command::new("runuser");
            cmd.arg("-u").arg(user).arg("--").arg(program);
            for a in args {
                cmd.arg(a);
            }
            cmd
        }
        None => {
            let mut cmd = std::process::Command::new(program);
            for a in args {
                cmd.arg(a);
            }
            cmd
        }
    }
}

fn params_object(params: &Value) -> Result<&Map<String, Value>, String> {
    params
        .as_object()
        .ok_or_else(|| "params must be an object".to_string())
}

fn str_param<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn bool_param(obj: &Map<String, Value>, key: &str, default: bool) -> bool {
    match obj.get(key) {
        None | Some(Value::Null) => default,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => {
            matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "on" | "1")
        }
        Some(other) => other.as_i64() == Some(1),
    }
}

/// Resolve a user name or numeric id against /etc/passwd (correct under
/// static musl, where NSS is unavailable by construction).
fn resolve_uid(owner: &str) -> Result<u32, String> {
    if let Ok(uid) = owner.parse::<u32>() {
        return Ok(uid);
    }
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    for line in passwd.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(owner) {
            let uid = fields.nth(1).ok_or("malformed passwd line")?;
            return uid.parse().map_err(|_| "malformed uid".to_string());
        }
    }
    Err(format!("user {owner:?} not found"))
}

fn resolve_gid(group: &str) -> Result<u32, String> {
    if let Ok(gid) = group.parse::<u32>() {
        return Ok(gid);
    }
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    for line in groups.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(group) {
            let gid = fields.nth(1).ok_or("malformed group line")?;
            return gid.parse().map_err(|_| "malformed gid".to_string());
        }
    }
    Err(format!("group {group:?} not found"))
}

/// Parse a mode param: octal string ("0755") or integer.
fn parse_mode(value: &Value) -> Result<u32, String> {
    match value {
        Value::String(s) => u32::from_str_radix(s.trim_start_matches("0o"), 8)
            .map_err(|_| format!("invalid mode {s:?}")),
        Value::Number(n) => {
            // YAML 0755 without quotes arrives as decimal 755 read as
            // octal-by-convention — Ansible warns and treats literally;
            // the workload always quotes modes, so a bare number here is
            // already-octal semantics from JSON round-trips.
            let raw = n.as_u64().ok_or("invalid numeric mode")?;
            u32::from_str_radix(&raw.to_string(), 8).map_err(|_| format!("invalid mode {raw}"))
        }
        other => Err(format!("invalid mode {other:?}")),
    }
}

/// chown/chmod attributes shared by file and copy.
fn apply_attrs(
    path: &std::path::Path,
    obj: &Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;

    if let Some(mode_v) = obj.get("mode") {
        let want = parse_mode(mode_v)?;
        if meta.permissions().mode() & 0o7777 != want {
            *changed = true;
            if !check_mode {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(want))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    let want_uid = match str_param(obj, "owner") {
        Some(o) => Some(resolve_uid(o)?),
        None => None,
    };
    let want_gid = match str_param(obj, "group") {
        Some(g) => Some(resolve_gid(g)?),
        None => None,
    };
    if want_uid.is_some_and(|u| u != meta.uid()) || want_gid.is_some_and(|g| g != meta.gid()) {
        *changed = true;
        if !check_mode {
            std::os::unix::fs::chown(path, want_uid, want_gid).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

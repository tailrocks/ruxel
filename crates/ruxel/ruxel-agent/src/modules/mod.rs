#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(
    clippy::let_underscore_must_use,
    clippy::match_same_arms,
    reason = "replaced by the Ruxel rewrite"
)]

mod attrs;
#[cfg(target_vendor = "apple")]
use attrs::apply_attrs_at;
#[cfg(test)]
use attrs::open_attrs;
use attrs::{apply_attrs, apply_attrs_fd};

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
mod mount;
mod postgresql;
mod replace;
mod shell;
mod slurp;
mod stat;
mod sysctl;
mod systemd;
mod timezone;
mod user;

use serde_json::{Map, Value, json};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct ExecContext {
    pub check_mode: bool,
    pub diff_mode: bool,
    pub no_log: bool,
    pub environment: Vec<(String, String)>,
    pub become_user: Option<String>,
}

pub(super) fn unified_diff(before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let mut out = String::from("--- before\n+++ after\n");
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    for line in &b {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub(crate) struct Outcome {
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

pub(crate) fn execute(
    module: &str,
    params: &Value,
    free_form: &str,
    ctx: &ExecContext,
    system_state: &mut crate::system_state::SystemState,
) -> Outcome {
    let result = match module {
        "filesystem" => filesystem::run(params, ctx),
        "git" => git::run(params, ctx),
        "iptables" => iptables::run(params, ctx),
        "lineinfile" => lineinfile::run(params, ctx),
        "community.general.lvol" => lvol::run(params, ctx),
        "ansible.posix.mount" => mount::run(params, ctx),
        "community.postgresql.postgresql_db" => postgresql::db(params, ctx),
        "community.postgresql.postgresql_user" => postgresql::user(params, ctx),
        "community.postgresql.postgresql_schema" => postgresql::schema(params, ctx),
        "community.postgresql.postgresql_privs" => postgresql::privs(params, ctx),
        "replace" => replace::run(params, ctx),
        "shell" => shell::run(params, free_form, ctx),
        "stat" => stat::run(params),
        "community.general.timezone" => timezone::run(params, ctx),
        "apt" => apt::run(params, ctx, system_state),
        "apt_repository" => apt_repository::run(params, ctx),
        "blockinfile" => blockinfile::run(params, ctx),
        "sysctl" | "ansible.posix.sysctl" => sysctl::run(params, ctx),
        "group" => user::group(params, ctx),
        "user" => user::run(params, ctx),
        "authorized_key" => authorized_key::run(params, ctx),
        "community.general.lvg" => lvg::run(params, ctx),
        "template" => copy::run(params, ctx),
        "get_url" => get_url::run(params, ctx),
        "command" => command::run(params, free_form, ctx),
        "file" => file::run(params, ctx),
        "copy" => copy::run(params, ctx),
        "slurp" => slurp::run(params),
        "systemd" | "service" => systemd::run(params, ctx, system_state),
        _ => Err(format!(
            "module {module:?} is not implemented in this agent build"
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

pub(crate) fn shutdown() {
    postgresql::shutdown();
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_with(path, bytes, None, None)
}

pub(super) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    owner: Option<(u32, u32)>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let existing = std::fs::symlink_metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file);
    let final_mode = mode
        .or_else(|| existing.as_ref().map(|m| m.permissions().mode() & 0o7777))
        .unwrap_or(0o600);
    let final_owner = owner.or_else(|| existing.as_ref().map(|m| (m.uid(), m.gid())));

    for _ in 0..100 {
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.ruxel-{}-{seq}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(final_mode)
            .open(&temp);
        let mut file = match opened {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let result = (|| {
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(final_mode))
                .map_err(|e| e.to_string())?;
            if let Some((uid, gid)) = final_owner {
                std::os::unix::fs::chown(&temp, Some(uid), Some(gid)).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return result;
    }
    Err(format!(
        "could not create atomic temporary file for {}",
        path.display()
    ))
}

pub(super) fn reject_newlines(module: &str, fields: &[(&str, &str)]) -> Result<(), String> {
    for (name, value) in fields {
        if value.contains(['\n', '\r']) {
            return Err(format!("{module}: {name} must be a single line"));
        }
    }
    Ok(())
}

pub(super) fn run_checked(
    mut command: std::process::Command,
) -> Result<std::process::Output, String> {
    let display = command_display(&command);
    let output = command
        .output()
        .map_err(|error| format!("exec {}: {error}", command.get_program().to_string_lossy()))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{display}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
pub(super) fn run_ok(mut command: std::process::Command) -> Result<bool, String> {
    command
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("exec {}: {error}", command.get_program().to_string_lossy()))
}

fn command_display(command: &std::process::Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn become_command(
    ctx: &ExecContext,
    program: &str,
    args: &[&str],
) -> std::process::Command {
    if let Some(user) = &ctx.become_user {
        let mut cmd = std::process::Command::new("runuser");
        cmd.arg("-u").arg(user).arg("--");
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd")
            && let Some((home, shell)) = user_environment_from(&passwd, user)
        {
            cmd.arg("/usr/bin/env")
                .arg(format!("HOME={home}"))
                .arg(format!("USER={user}"))
                .arg(format!("LOGNAME={user}"))
                .arg(format!("SHELL={shell}"));
        }
        cmd.arg(program);
        for a in args {
            cmd.arg(a);
        }
        cmd
    } else {
        let mut cmd = std::process::Command::new(program);
        for a in args {
            cmd.arg(a);
        }
        cmd
    }
}

fn user_environment_from<'a>(passwd: &'a str, user: &str) -> Option<(&'a str, &'a str)> {
    passwd.lines().find_map(|line| {
        let fields = line.split(':').collect::<Vec<_>>();
        (fields.len() >= 7 && fields[0] == user).then_some((fields[5], fields[6]))
    })
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

fn resolve_uid(owner: &str) -> Result<u32, String> {
    if let Ok(uid) = owner.parse::<u32>() {
        return Ok(uid);
    }
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    resolve_uid_from(&passwd, owner)
}

fn resolve_uid_from(passwd: &str, owner: &str) -> Result<u32, String> {
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
    resolve_gid_from(&groups, group)
}

fn resolve_gid_from(groups: &str, group: &str) -> Result<u32, String> {
    for line in groups.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(group) {
            let gid = fields.nth(1).ok_or("malformed group line")?;
            return gid.parse().map_err(|_| "malformed gid".to_string());
        }
    }
    Err(format!("group {group:?} not found"))
}

pub(super) fn parse_mode(value: &Value) -> Result<u32, String> {
    match value {
        Value::String(s) => u32::from_str_radix(s.trim_start_matches("0o"), 8)
            .map_err(|_| format!("invalid mode {s:?}")),
        Value::Number(n) => {
            let raw = n.as_u64().ok_or("invalid numeric mode")?;
            u32::from_str_radix(&raw.to_string(), 8).map_err(|_| format!("invalid mode {raw}"))
        }
        other => Err(format!("invalid mode {other:?}")),
    }
}

pub(super) fn default_file_mode(status: &str) -> u32 {
    let umask = status
        .lines()
        .find_map(|line| line.strip_prefix("Umask:\t"))
        .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
        .unwrap_or(0o022);
    0o666 & !umask
}

#[cfg(test)]
#[path = "tests/security.rs"]
mod security_tests;

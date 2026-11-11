#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]

mod attrs;
#[cfg(target_vendor = "apple")]
use attrs::apply_attrs_at;
#[cfg(test)]
use attrs::open_attrs;
use attrs::{apply_attrs, apply_attrs_fd, apply_attrs_with};

mod apt;
mod apt_repository;
mod atomic;
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
mod subprocess;
mod sysctl;
mod systemd;
mod timezone;
mod user;

use serde_json::{Map, Value, json};
use subprocess::become_command;

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

pub(crate) use copy::validate_source;

pub(crate) fn execute(
    module: &str,
    params: &Value,
    free_form: &str,
    source: Option<&[u8]>,
    ctx: &ExecContext,
    system_state: &mut crate::system_state::SystemState,
) -> Outcome {
    if ctx.become_user.is_some()
        && !matches!(
            module,
            "command"
                | "shell"
                | "postgres_database"
                | "postgres_role"
                | "postgres_schema"
                | "postgres_privileges"
        )
    {
        return Outcome::from_result(json!({
            "failed": true,
            "changed": false,
            "msg": format!("module {module:?} does not support become_user"),
        }));
    }
    let result = match module {
        "filesystem" => filesystem::run(params, ctx),
        "git" => git::run(params, ctx),
        "iptables" => iptables::run(params, ctx),
        "lineinfile" => lineinfile::run(params, ctx),
        "logical_volume" => lvol::run(params, ctx),
        "mount" => mount::run(params, ctx),
        "postgres_database" => postgresql::db(params, ctx),
        "postgres_role" => postgresql::user(params, ctx),
        "postgres_schema" => postgresql::schema(params, ctx),
        "postgres_privileges" => postgresql::privs(params, ctx),
        "replace" => replace::run(params, ctx),
        "shell" => shell::run(params, free_form, ctx),
        "stat" => stat::run(params),
        "timezone" => timezone::run(params, ctx),
        "apt" => apt::run(params, ctx, system_state),
        "apt_repository" => apt_repository::run(params, ctx, system_state),
        "blockinfile" => blockinfile::run(params, ctx),
        "sysctl" => sysctl::run(params, ctx),
        "group" => user::group(params, ctx),
        "user" => user::run(params, ctx),
        "authorized_key" => authorized_key::run(params, ctx),
        "volume_group" => lvg::run(params, ctx),
        "template" => copy::run(params, None, ctx),
        "get_url" => get_url::run(params, ctx),
        "command" if !free_form.is_empty() => {
            Err("command requires argv, not free-form input".into())
        }
        "command" => command::run(params, ctx),
        "file" => file::run(params, ctx),
        "copy" => copy::run(params, source, ctx),
        "slurp" => slurp::run(params),
        "systemd" => systemd::run(params, ctx, system_state),
        _ => Err(format!(
            "module {module:?} is not implemented in this agent build"
        )),
    };
    // A command or shell task can install packages or start, stop, enable
    // and disable units without the agent seeing it. Later apt and systemd
    // tasks, and the ledger probes that let them skip, must read the host
    // again instead of trusting the snapshot taken before it ran.
    if matches!(module, "command" | "shell") && !ctx.check_mode {
        system_state.invalidate_packages();
        system_state.invalidate_units();
    }
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

pub(super) use atomic::{write_atomic, write_atomic_with};

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
    #[cfg(test)]
    count_ownership_resolution();
    if let Ok(uid) = owner.parse::<u32>() {
        return Ok(uid);
    }
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    resolve_uid_from(&passwd, owner)
}

/// The requested ownership of a file, resolved to ids.
pub(super) struct Ownership {
    pub(super) uid: Option<u32>,
    pub(super) gid: Option<u32>,
}

/// Resolves the requested owner and group of `obj` to ids.
///
/// The ids are resolved once and reused by the caller: a change to the
/// passwd or group file between the resolution and the use of the ids
/// cannot fail or mis-apply a publication that already succeeded.
pub(super) fn resolve_ownership(obj: &Map<String, Value>) -> Result<Ownership, String> {
    Ok(Ownership {
        uid: str_param(obj, "owner").map(resolve_uid).transpose()?,
        gid: str_param(obj, "group").map(resolve_gid).transpose()?,
    })
}

// The number of ownership resolutions of the current thread.
//
// The counter is a test aid: a module that resolves the requested
// ownership once must not resolve it again.
#[cfg(test)]
thread_local! {
    pub(super) static OWNERSHIP_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts one ownership resolution of the current thread.
#[cfg(test)]
fn count_ownership_resolution() {
    OWNERSHIP_RESOLUTIONS.with(|count| count.set(count.get() + 1));
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
    #[cfg(test)]
    count_ownership_resolution();
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

#[cfg(test)]
#[path = "tests/systemd_wire.rs"]
mod systemd_wire_tests;

#[cfg(test)]
#[path = "tests/state_invalidation.rs"]
mod state_invalidation_tests;

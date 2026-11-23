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
mod debconf;
mod file;
mod filesystem;
mod get_url;
mod git;
mod iptables;
mod lineinfile;
mod lvg;
mod lvol;
mod mount;
mod ownership;
mod postgresql;
mod replace;
mod shell;
mod slurp;
mod stat;
mod subprocess;
mod sysctl;
pub(crate) mod systemd;
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
#[cfg(test)]
pub(super) use ownership::{OWNERSHIP_RESOLUTIONS, resolve_gid_from, resolve_uid_from};
pub(super) use ownership::{Ownership, resolve_ownership};
pub(crate) use sysctl::{
    ManagedSysctl, ManagedValue, SysctlManaged, boot_fingerprint, declaration_digest, entry_key,
    proc_sys_path, sysctl_cache_covered, sysctl_host_files, sysctl_key, trimmed_value,
};
pub(crate) use user::begin_plan;

/// The sysctl declaration the current plan's task carries: the file it
/// declares the key in, the key, and the value it declared there. The
/// plan declares the entry whatever the run does with the task — a cache
/// hit and a check-mode run declare too — so a later task's preflight
/// reads the plan's own entries, not the ledger's retained history of a
/// task the workflow dropped. A task that failed to parse declares
/// nothing: its entry is unknown.
pub(crate) fn sysctl_plan_declaration(module: &str, params: &Value) -> Option<ManagedSysctl> {
    if module != "sysctl" {
        return None;
    }
    let name = params.get("name").and_then(Value::as_str)?;
    let value = match params.get("value")? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => if *value { "1" } else { "0" }.to_string(),
        _ => return None,
    };
    let file = params
        .get("sysctl_file")
        .and_then(Value::as_str)
        .unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
    Some(ManagedSysctl::plain(
        file.to_string(),
        name.to_string(),
        value,
    ))
}

/// The managed sysctl entries a task's module consumes: only the
/// sysctl module reads them, and the scan they need weighs on every
/// persisted record — so a workflow of ordinary tasks would pay
/// 1+2+…+N inspections for a result none of them reads. The gate
/// keeps the scan to the module that needs it.
#[cfg(test)]
pub(crate) fn managed_sysctl_entries(
    module: &str,
    ledger: &crate::ledger::Ledger,
) -> Vec<ManagedSysctl> {
    if module == "sysctl" {
        ledger.managed_sysctl_entries()
    } else {
        Vec::new()
    }
}

/// Whether the plan's own earlier steps moved the number of an
/// identity this task names, so the task's cached result is stale.
///
/// `usermod -u` renumbers the account but leaves an arbitrary file at
/// the old number, and the cached probe of a file weighs the file's
/// numbers alone: the task would be served from the cache and skipped,
/// and the file would keep the number the plan gave the account. The
/// task names the identity whose number moved, so the plan's own
/// record of it is what tells the cache is stale.
pub(crate) fn ownership_cache_stale(module: &str, params: &Value) -> bool {
    if !matches!(
        module,
        "file" | "copy" | "template" | "lineinfile" | "replace" | "blockinfile"
    ) {
        return false;
    }
    let owner = params.get("owner").and_then(Value::as_str);
    let group = params.get("group").and_then(Value::as_str);
    owner.is_some_and(user::user_number_changed) || group.is_some_and(user::group_number_changed)
}

pub(crate) fn execute(
    module: &str,
    params: &Value,
    free_form: &str,
    source: Option<&[u8]>,
    ctx: &ExecContext,
    system_state: &mut crate::system_state::SystemState,
    managed: &SysctlManaged,
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
        "sysctl" => sysctl::run(params, ctx, managed),
        "group" => user::group(params, ctx),
        "user" => user::run(params, ctx),
        "authorized_key" => authorized_key::run(params, ctx),
        "volume_group" => lvg::run(params, ctx),
        "template" => copy::run(params, None, ctx, system_state),
        "get_url" => get_url::run(params, ctx),
        "command" if !free_form.is_empty() => {
            Err("command requires argv, not free-form input".into())
        }
        "command" => command::run(params, ctx),
        "file" => file::run(params, ctx),
        "copy" => copy::run(params, source, ctx, system_state),
        "debconf" => debconf::run(params, ctx),
        "slurp" => slurp::run(params),
        // A FetchSecret task is the controller's no-log slurp: the agent
        // reads the file, the controller stores it in fnox. The controller
        // sends it in apply only.
        "fetchSecret" => match params.get("path").and_then(Value::as_str) {
            Some(path) => slurp::run(&json!({"src": path})),
            None => Err("fetchSecret: path required".to_owned()),
        },
        "systemd" => systemd::run(params, ctx, system_state),
        _ => Err(format!(
            "module {module:?} is not implemented in this agent build"
        )),
    };
    // A command or shell task can install packages or start, stop, enable
    // and disable units without the agent seeing it. Later apt and systemd
    // tasks, and the ledger probes that let them skip, must read the host
    // again instead of trusting the snapshot taken before it ran — and no
    // prediction survives it either.
    if matches!(module, "command" | "shell") && !ctx.check_mode {
        system_state.invalidate_packages();
        system_state.invalidate_units();
        system_state.clear_predictions();
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

/// A value the operating system cannot carry: a NUL cannot be in a
/// process argument at all, so a value that holds one is refused before
/// it reaches a spawn.
pub(super) fn reject_nuls(module: &str, fields: &[(&str, &str)]) -> Result<(), String> {
    for (name, value) in fields {
        if value.contains('\0') {
            return Err(format!("{module}: {name} must not hold a NUL byte"));
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
#[path = "tests/managed_entries.rs"]
mod managed_entries_tests;

#[cfg(test)]
#[path = "tests/security.rs"]
mod security_tests;

#[cfg(test)]
#[path = "tests/systemd_wire.rs"]
mod systemd_wire_tests;

#[cfg(test)]
#[path = "tests/state_invalidation.rs"]
mod state_invalidation_tests;

#[cfg(test)]
#[path = "tests/template_invalidation.rs"]
mod template_invalidation_tests;

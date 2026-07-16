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
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ExecContext {
    pub check_mode: bool,
    /// `--diff`: content modules embed a unified diff in their result.
    pub diff_mode: bool,
    /// `no_log`: sensitive task output must not be duplicated into diffs.
    pub no_log: bool,
    /// Task `environment:` merged into the child process env.
    pub environment: Vec<(String, String)>,
    /// `become_user:` — run module subprocesses as this user (SEMANTICS §1).
    pub become_user: Option<String>,
}

/// A minimal unified diff (before → after) for content modules under
/// `--diff`. Whole-file, no hunking — matches what operators expect to
/// eyeball for a config change and keeps the agent dependency-free.
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

pub fn execute(
    module: &str,
    params: &Value,
    free_form: &str,
    ctx: &ExecContext,
    system_state: &mut crate::system_state::SystemState,
) -> Outcome {
    if !is_implemented(module) {
        return Outcome::from_result(json!({
            "failed": true,
            "changed": false,
            "msg": format!("module {module:?} is not implemented in this agent build"),
        }));
    }
    let result = match module {
        "apt" => apt::run(params, ctx, system_state),
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
        "systemd" | "service" => systemd::run(params, ctx, system_state),
        "community.postgresql.postgresql_db" => postgresql::db(params, ctx),
        "community.postgresql.postgresql_user" => postgresql::user(params, ctx),
        "community.postgresql.postgresql_schema" => postgresql::schema(params, ctx),
        "community.postgresql.postgresql_privs" => postgresql::privs(params, ctx),
        _ => unreachable!("is_implemented and execute dispatch must stay aligned"),
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

pub fn is_implemented(module: &str) -> bool {
    matches!(
        module,
        "ansible.posix.mount"
            | "ansible.posix.sysctl"
            | "apt"
            | "apt_repository"
            | "authorized_key"
            | "blockinfile"
            | "command"
            | "community.general.lvg"
            | "community.general.lvol"
            | "community.general.timezone"
            | "community.postgresql.postgresql_db"
            | "community.postgresql.postgresql_privs"
            | "community.postgresql.postgresql_schema"
            | "community.postgresql.postgresql_user"
            | "copy"
            | "file"
            | "filesystem"
            | "get_url"
            | "git"
            | "group"
            | "iptables"
            | "lineinfile"
            | "replace"
            | "service"
            | "shell"
            | "slurp"
            | "stat"
            | "sysctl"
            | "systemd"
            | "template"
            | "user"
    )
}

pub fn shutdown() {
    postgresql::shutdown();
}

// -- Shared helpers -----------------------------------------------------------

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_with(path, bytes, None, None)
}

/// Same-directory exclusive temp plus rename: never follows the destination.
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
    let existing = std::fs::symlink_metadata(path).ok().filter(|m| m.is_file());
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

/// Resolve a user name or numeric id against /etc/passwd (correct under
/// static musl, where NSS is unavailable by construction).
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

#[cfg(test)]
mod security_tests {
    use super::{
        bool_param, parse_mode, resolve_gid_from, resolve_uid_from, user_environment_from,
        write_atomic,
    };
    use serde_json::json;
    use std::os::unix::fs::symlink;

    #[test]
    fn atomic_write_replaces_symlink_not_its_target() {
        let root = std::env::temp_dir().join(format!("ruxel-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let victim = root.join("victim");
        let dest = root.join("dest");
        std::fs::write(&victim, "safe").unwrap();
        symlink(&victim, &dest).unwrap();
        write_atomic(&dest, b"replacement").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "replacement");
        assert!(
            !std::fs::symlink_metadata(&dest)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_modes_booleans_and_identity_files() {
        assert_eq!(parse_mode(&json!("0755")).unwrap(), 0o755);
        assert_eq!(parse_mode(&json!(644)).unwrap(), 0o644);
        for value in [
            json!("yes"),
            json!("true"),
            json!("on"),
            json!("1"),
            json!(1),
        ] {
            let obj = json!({"value": value});
            assert!(bool_param(obj.as_object().unwrap(), "value", false));
        }
        assert_eq!(
            resolve_uid_from("alice:x:1001:1002::/home/alice:/bin/sh\n", "alice").unwrap(),
            1001
        );
        assert_eq!(
            resolve_gid_from("staff:x:1002:alice\n", "staff").unwrap(),
            1002
        );
        assert_eq!(
            user_environment_from("alice:x:1001:1002::/home/alice:/bin/sh\n", "alice"),
            Some(("/home/alice", "/bin/sh"))
        );
    }

    #[test]
    fn checked_runner_preserves_program_args_and_stderr() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "printf synthetic-error >&2; exit 7"]);
        let error = super::run_checked(command).unwrap_err();
        assert!(error.starts_with("sh -c printf synthetic-error >&2; exit 7:"));
        assert!(error.ends_with("synthetic-error"));

        let mut ok = std::process::Command::new("sh");
        ok.args(["-c", "exit 0"]);
        assert!(super::run_ok(ok).unwrap());
    }

    #[test]
    fn every_remote_core_module_has_agent_dispatch() {
        const CONTROLLER_ONLY: &[&str] = &["assert", "debug", "fail", "pause", "set_fact"];
        let missing: Vec<_> = ruxel_core::modules::MODULES
            .iter()
            .map(|surface| surface.name)
            .filter(|name| !CONTROLLER_ONLY.contains(name))
            .filter(|name| !super::is_implemented(name))
            .collect();
        assert!(
            missing.is_empty(),
            "core modules missing agent dispatch: {missing:?}"
        );
        let context = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        for module in ruxel_core::modules::MODULES
            .iter()
            .map(|surface| surface.name)
            .filter(|name| !CONTROLLER_ONLY.contains(name))
        {
            let outcome = super::execute(
                module,
                &serde_json::Value::Null,
                "",
                &context,
                &mut crate::system_state::SystemState::default(),
            );
            assert!(
                !outcome.result["msg"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("not implemented"),
                "{module} is advertised but has no execute arm"
            );
        }
    }
}

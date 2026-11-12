use self::holds::{Holds, hold_packages, read_showhold};
use super::{ExecContext, bool_param, params_object, str_param};
use crate::system_state::{PackageRequest, SystemState};
use serde_json::{Map, Value, json};

type CommandResult = Result<(String, i32, String), String>;

pub(super) fn run(
    params: &Value,
    ctx: &ExecContext,
    system_state: &mut SystemState,
) -> Result<Value, String> {
    run_with(params, ctx, system_state, &mut apt_command)
}

fn run_with(
    params: &Value,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let update_cache = bool_param(obj, "update_cache", false);
    let upgrade = str_param(obj, "upgrade");
    let autoremove = bool_param(obj, "autoremove", false);
    let hold = bool_param(obj, "hold", false);
    let requests = check_requests(obj, ctx)?;
    let state = str_param(obj, "state").unwrap_or("present");
    let mut result = json!({"changed": false});
    let mut changed = false;
    if update_cache {
        match refresh_cache(ctx, system_state, &mut result, command) {
            // `None` is a failed refresh: the result carries the failure.
            None => return Ok(result),
            Some(refreshed) => changed |= refreshed,
        }
    }
    if let Some(kind) = upgrade {
        let action = match kind {
            "dist" => "dist-upgrade",
            other => return Err(format!("apt: upgrade={other:?} outside the closed surface")),
        };
        // In check and diff mode the upgrade and the autoremove are
        // simulated as one transaction (`--auto-remove`): a separate
        // `-s autoremove` after a simulated upgrade would evaluate the
        // pre-upgrade dependency graph — the simulation changes nothing —
        // while apply mode runs the autoremove after the real upgrade.
        // Check and apply must agree on the removals.
        let combined = autoremove && ctx.check_mode;
        match maintenance(
            action,
            if combined { &["--auto-remove"] } else { &[] },
            true,
            ctx,
            system_state,
            &mut result,
            command,
        ) {
            Ok(more) => changed |= more,
            Err(error) => return Ok(failed(&mut result, changed, &error)),
        }
        if combined {
            result["diff"] = json!({});
        }
    }
    if autoremove && !(upgrade.is_some() && ctx.check_mode) {
        match maintenance("autoremove", &[], false, ctx, system_state, &mut result, command) {
            Ok(more) => changed |= more,
            Err(error) => return Ok(failed(&mut result, changed, &error)),
        }
        result["diff"] = json!({});
    }
    // The holds are read before anything runs: a package that is already
    // held is not held again, and the report names the holds of the host.
    let holds = if hold {
        Some(read_showhold(command, ctx)?)
    } else {
        None
    };
    if !requests.is_empty() {
        match install(
            &requests,
            holds.as_deref().unwrap_or_default(),
            state,
            obj,
            ctx,
            system_state,
            &mut result,
            command,
        )? {
            Installed::Unchanged => {}
            Installed::Changed => changed = true,
            Installed::Unverified(message) => {
                // apt-get succeeded, so the host may have changed; keep its
                // output alongside the failure.
                result["changed"] = json!(true);
                result["failed"] = json!(true);
                result["msg"] = json!(message);
                return Ok(result);
            }
        }
    }
    if let Some(holds) = holds {
        let native = system_state.native_architecture().map(str::to_owned);
        match hold_packages(
            holds,
            &requests,
            native.as_deref(),
            ctx,
            &mut result,
            command,
        ) {
            Ok(Holds::Applied(marked)) => changed |= marked,
            // The report carries the failure; the step ends there.
            Ok(Holds::Unverified) => return Ok(result),
            // A hold that fails after the run changed the host must not
            // hide the change behind an error: `modules::execute` turns an
            // Err into `changed: false`. The report keeps the change the
            // run accumulated (true after an install) alongside the
            // failure, like the Unverified arm above.
            Err(error) => return Ok(failed(&mut result, changed, &error)),
        }
    }
    result["changed"] = json!(changed);
    result["failed"] = json!(false);
    Ok(result)
}

/// The result of a failure that stops the run after the host may already
/// have changed: the change the run accumulated stays in the report
/// alongside the failure and its message, because `modules::execute`
/// would turn an Err into `changed: false` and hide that change.
fn failed(result: &mut Value, changed: bool, error: &str) -> Value {
    result["changed"] = json!(changed);
    result["failed"] = json!(true);
    result["msg"] = json!(error);
    std::mem::take(result)
}

/// The packages an apt step asks for, and the checks that must hold before
/// any command runs.
///
/// # Errors
/// Returns an error naming the refused request or the refused combination.
fn check_requests(
    obj: &Map<String, Value>,
    ctx: &ExecContext,
) -> Result<Vec<PackageRequest>, String> {
    let requests = PackageRequest::parse_all(&package_names(obj)?)?;
    if ctx.environment.iter().any(|(key, _)| key == "APT_CONFIG") {
        return Err(
            "apt: APT_CONFIG in the task environment is not supported; package state is read from the host's dpkg database".into(),
        );
    }
    if str_param(obj, "upgrade").is_some()
        && requests.iter().any(|request| request.version().is_some())
    {
        return Err(
            "apt: upgrade cannot be combined with exact version pins, which the upgrade would move"
                .into(),
        );
    }
    if bool_param(obj, "hold", false) && str_param(obj, "state").unwrap_or("present") == "latest" {
        return Err(
            "apt: hold cannot be combined with state=latest; a held package is never upgraded"
                .into(),
        );
    }
    Ok(requests)
}

/// Refreshes the package lists of the host. The time of the refresh is
/// reported, and the agent's package snapshot is invalidated, so that a
/// later package step reads the new lists.
///
/// `None` is a failed refresh: the result carries the failure, and the
/// step ends there. `Some` says whether the lists were refreshed: in check
/// mode nothing runs, so they were not.
fn refresh_cache(
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Option<bool> {
    if !ctx.check_mode {
        let out = command(&["update"], ctx);
        system_state.invalidate_packages();
        let out = match out {
            Ok(out) => out,
            Err(error) => {
                *result = json!({"failed": true, "changed": true, "msg": error});
                return None;
            }
        };
        if out.1 != 0 {
            *result = json!({
                "failed": true,
                "changed": true,
                "msg": format!("apt update failed: {}", out.2),
            });
            return None;
        }
    }
    result["cache_updated"] = json!(!ctx.check_mode);
    result["cache_update_time"] = json!(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    );
    Some(!ctx.check_mode)
}

fn package_names(obj: &Map<String, Value>) -> Result<Vec<String>, String> {
    match obj.get("name") {
        None => Ok(vec![]),
        Some(Value::String(name)) => Ok(vec![name.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "apt: name entries must be strings".to_string())
            })
            .collect(),
        Some(other) => Err(format!("apt: invalid name {other:?}")),
    }
}

fn maintenance(
    action: &str,
    extra: &[&str],
    with_msg: bool,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<bool, String> {
    let mut args = if ctx.check_mode {
        vec!["-s"]
    } else {
        vec![]
    };
    args.extend_from_slice(extra);
    args.push(action);
    let (stdout, rc, stderr) = command(&args, ctx)?;
    if rc != 0 {
        let simulation = if ctx.check_mode { " -s" } else { "" };
        return Err(format!("apt {action}{simulation} failed: {stderr}"));
    }
    if !ctx.check_mode {
        system_state.invalidate_packages();
    }
    fill_exec_fields(result, &stdout, &stderr, with_msg);
    Ok(summary_changed(&stdout))
}

enum Installed {
    Unchanged,
    Changed,
    /// apt-get succeeded but the requests are not all satisfied afterwards.
    Unverified(String),
}

fn install(
    requests: &[PackageRequest],
    held: &[PackageRequest],
    state: &str,
    obj: &Map<String, Value>,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Installed, String> {
    let latest = state == "latest";
    let plan = system_state.plan_packages(requests, latest, &ctx.environment)?;
    if plan.missing.is_empty() {
        return Ok(Installed::Unchanged);
    }
    if !ctx.check_mode {
        let mut args = vec!["install"];
        if bool_param(obj, "force", false) {
            args.push("--allow-downgrades");
        }
        // A `hold` step whose pin changed must move the packages the host
        // already holds. Only the requested holds are lifted for this
        // transaction and restored right after; every other held package
        // keeps apt's refusal to move it. The transaction-wide
        // `--allow-change-held-packages` is not used: apt documents it as
        // a dangerous force-yes that lets the run change any held
        // package, even one the workflow never requested.
        let native = system_state.native_architecture().map(str::to_owned);
        let lifted: Vec<String> = if bool_param(obj, "hold", false) {
            requests
                .iter()
                .filter(|request| held.iter().any(|entry| request.covered_by(entry, native.as_deref())))
                .map(PackageRequest::selector)
                .collect()
        } else {
            Vec::new()
        };
        if !lifted.is_empty() {
            let mut marks = vec!["unhold"];
            marks.extend(lifted.iter().map(String::as_str));
            let (_, rc, stderr) = command(&marks, ctx)?;
            if rc != 0 {
                return Err(format!("apt-mark unhold failed: {stderr}"));
            }
        }
        args.extend(plan.transaction.iter().map(String::as_str));
        let (stdout, rc, stderr) = command(&args, ctx)?;
        system_state.invalidate_packages();
        // The holds are restored whatever the transaction answered: the
        // host held the packages before the step, so the step leaves them
        // held — or reports the mark that could not be restored.
        let restored = restore_holds(&lifted, command, ctx);
        if rc != 0 {
            return Err(format!("apt install failed: {stderr}"));
        }
        if let Err(error) = restored {
            return Ok(Installed::Unverified(format!(
                "apt install succeeded but the hold could not be restored: {error}"
            )));
        }
        fill_exec_fields(result, &stdout, &stderr, false);
        match system_state.plan_packages(requests, latest, &ctx.environment) {
            Ok(after) if after.missing.is_empty() => {}
            Ok(after) => {
                return Ok(Installed::Unverified(format!(
                    "apt install did not satisfy the requested packages: {}",
                    after.missing.join(", ")
                )));
            }
            Err(error) => {
                return Ok(Installed::Unverified(format!(
                    "apt install succeeded but verification failed: {error}"
                )));
            }
        }
    }
    Ok(Installed::Changed)
}

/// Puts back the holds the transaction lifted, so the host keeps the
/// state the step promised.
fn restore_holds(
    lifted: &[String],
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
    ctx: &ExecContext,
) -> Result<(), String> {
    if lifted.is_empty() {
        return Ok(());
    }
    let mut marks = vec!["hold"];
    marks.extend(lifted.iter().map(String::as_str));
    let (_, rc, stderr) = command(&marks, ctx)?;
    if rc != 0 {
        return Err(format!("apt-mark hold failed: {stderr}"));
    }
    Ok(())
}

/// apt-get and apt-mark share one command: the arguments name the
/// operation, so `showhold`, `hold` and `unhold` run apt-mark and every
/// other argument runs apt-get.
fn apt_command(args: &[&str], ctx: &ExecContext) -> CommandResult {
    match args.first() {
        Some(&"showhold" | &"hold" | &"unhold") => apt_mark(args, ctx),
        _ => apt_get(args, ctx),
    }
}

/// `apt-mark` asks no questions and changes nothing but the marks, so it
/// needs neither `-y` nor the dpkg options apt-get carries.
fn apt_mark(args: &[&str], ctx: &ExecContext) -> CommandResult {
    let mut cmd = std::process::Command::new("apt-mark");
    cmd.args(args);
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    force_stable_locale(&mut cmd);
    let out = cmd.output().map_err(|e| format!("exec apt-mark: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn apt_get(args: &[&str], ctx: &ExecContext) -> CommandResult {
    let mut cmd = std::process::Command::new("apt-get");
    cmd.arg("-y")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confdef")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confold");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    force_stable_locale(&mut cmd);
    let out = cmd.output().map_err(|e| format!("exec apt-get: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

/// A stable locale for the apt subprocesses: `summary_changed` recognizes
/// only the English summary strings (`upgraded,`, `newly installed`), and
/// apt-get prints translated prose under a non-C `LANG` or `LC_MESSAGES`,
/// so a successful localized run would report no change. Forced after the
/// task environment, like the `apt-cache` reader of the package state.
fn force_stable_locale(cmd: &mut std::process::Command) {
    cmd.env("LC_ALL", "C");
}

fn summary_changed(stdout: &str) -> bool {
    for line in stdout.lines() {
        if line.contains("upgraded,") && line.contains("newly installed") {
            let numbers: Vec<u64> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            return numbers.iter().take(3).any(|n| *n > 0);
        }
    }
    false
}

fn fill_exec_fields(result: &mut Value, stdout: &str, stderr: &str, with_msg: bool) {
    let stdout = stdout.trim_end_matches('\n');
    let stderr = stderr.trim_end_matches('\n');
    let lines = |s: &str| -> Vec<String> {
        if s.is_empty() {
            vec![]
        } else {
            s.lines().map(str::to_string).collect()
        }
    };
    if with_msg {
        result["msg"] = json!(stdout);
    }
    result["stdout"] = json!(stdout);
    result["stderr"] = json!(stderr);
    result["stdout_lines"] = json!(lines(stdout));
    result["stderr_lines"] = json!(lines(stderr));
}

mod holds;

#[cfg(test)]
mod tests;

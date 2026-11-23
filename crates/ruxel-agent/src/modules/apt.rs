use self::commands::{apt_command, fill_exec_fields, summary_changed};
use self::holds::{Holds, hold_packages, read_showhold};
use self::install::install;
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

#[expect(
    clippy::too_many_lines,
    reason = "the run is one straight-line transaction and one report; a split would scatter one result across helpers"
)]
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
        // A pre-existing hold — manual or residual — makes the
        // whole-system result undeliverable: a plain `dist-upgrade` runs
        // without `--ignore-hold` and leaves the held package below its
        // candidate, and the only option that moves it is the dangerous
        // force override apt-get(8) documents. Refuse instead and name
        // the holds, in check and apply alike — the operator unholds the
        // package or removes the upgrade step — as the Lowering refuses
        // a hold declared beside an upgrade.
        // The discovery failure carries the change the run accumulated:
        // a completed `apt-get update` before it must not be hidden
        // behind an error `modules::execute` reports as `changed: false`.
        let preexisting = match read_showhold(command, ctx) {
            Ok(preexisting) => preexisting,
            Err(error) => return Ok(failed(&mut result, changed, &error)),
        };
        if !preexisting.is_empty() {
            return Ok(failed(
                &mut result,
                changed,
                &format!(
                    "apt: the host holds {}; a system upgrade cannot leave a held package behind — unhold the package or remove the upgrade step",
                    preexisting
                        .iter()
                        .map(PackageRequest::selector)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
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
            Err((maybe_changed, error)) => {
                return Ok(failed(&mut result, changed || maybe_changed, &error));
            }
        }
        if combined {
            result["diff"] = json!({});
        }
    }
    if autoremove && !(upgrade.is_some() && ctx.check_mode) {
        match maintenance(
            "autoremove",
            &[],
            false,
            ctx,
            system_state,
            &mut result,
            command,
        ) {
            Ok(more) => changed |= more,
            Err((maybe_changed, error)) => {
                return Ok(failed(&mut result, changed || maybe_changed, &error));
            }
        }
        result["diff"] = json!({});
    }
    // The holds are read before anything runs: a package that is already
    // held is not held again, and the report names the holds of the host.
    let holds = if hold {
        // The same treatment as the upgrade's discovery above: the
        // failure keeps the change the refresh, upgrade or autoremove
        // already made beside it.
        match read_showhold(command, ctx) {
            Ok(holds) => Some(holds),
            Err(error) => return Ok(failed(&mut result, changed, &error)),
        }
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
        ) {
            Ok(Installed::Unchanged) => {}
            Ok(Installed::Changed) => changed = true,
            Ok(Installed::Unverified(message)) => {
                // apt-get succeeded, so the host may have changed; keep its
                // output alongside the failure.
                result["changed"] = json!(true);
                result["failed"] = json!(true);
                result["msg"] = json!(message);
                return Ok(result);
            }
            // A transaction that failed after the holds were lifted may
            // have left the package unintentionally unheld: the error
            // carries that known change, and `modules::execute` would
            // turn an Err into `changed: false` and hide it.
            Err((maybe_changed, error)) => {
                return Ok(failed(&mut result, changed || maybe_changed, &error));
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

/// Runs one maintenance action and says whether it changed the host. The
/// error carries whether the host may have changed: a failed apply-mode
/// run mutates dpkg before it stops, and `modules::execute` turns an Err
/// into `changed: false`, so the flag keeps that change visible.
fn maintenance(
    action: &str,
    extra: &[&str],
    with_msg: bool,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<bool, (bool, String)> {
    let mut args = if ctx.check_mode { vec!["-s"] } else { vec![] };
    args.extend_from_slice(extra);
    args.push(action);
    // A transport error means apt-get never started, so nothing changed.
    let (stdout, rc, stderr) = command(&args, ctx).map_err(|error| (false, error))?;
    if rc != 0 {
        let simulation = if ctx.check_mode { " -s" } else { "" };
        // A failed apply-mode run may have changed the host before it
        // stopped: drop the package snapshot, so no later step plans
        // against it, and report the change alongside the failure.
        let maybe_changed = !ctx.check_mode;
        if maybe_changed {
            system_state.invalidate_packages();
            // A maintenance run can stop services and autoremove their
            // units: the unit snapshot goes with the package snapshot.
            system_state.invalidate_units();
        }
        return Err((
            maybe_changed,
            format!("apt {action}{simulation} failed: {stderr}"),
        ));
    }
    if !ctx.check_mode {
        system_state.invalidate_packages();
        // A maintenance run can stop services and autoremove their
        // units: a later service step must read the host again instead of
        // trusting cached `active` or `enabled` values.
        system_state.invalidate_units();
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

mod commands;
mod holds;
mod install;

#[cfg(test)]
mod tests;

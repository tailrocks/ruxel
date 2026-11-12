//! The installation of one apt step: the transaction apt-get runs and
//! the holds it lifts and restores for it.

use super::{CommandResult, ExecContext, bool_param, fill_exec_fields, str_param};
use crate::system_state::{PackageRequest, SystemState};
use serde_json::{Map, Value, json};

pub(super) enum Installed {
    Unchanged,
    Changed,
    /// apt-get succeeded but the requests are not all satisfied afterwards.
    Unverified(String),
}

pub(super) fn install(
    requests: &[PackageRequest],
    held: &[PackageRequest],
    obj: &Map<String, Value>,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Installed, String> {
    let latest = str_param(obj, "state") == Some("latest");
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
                .filter(|request| {
                    held.iter()
                        .any(|entry| request.covered_by(entry, native.as_deref()))
                })
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

//! The install transaction of an apt step: the plan's missing packages
//! are installed, and a `hold` step's own pins may move packages the host
//! already holds — the holds are lifted for that transaction alone and
//! restored right after, whatever it answers.

use super::commands::{fill_exec_fields, summary_changed};
use super::holds::restore_holds;
use super::{CommandResult, ExecContext, Installed, bool_param};
use crate::system_state::{PackageRequest, SystemState};
use serde_json::{Map, Value};

#[expect(
    clippy::too_many_arguments,
    reason = "the install step carries the request, the transaction knobs and the shared report; a context struct would hide which of it changes"
)]
pub(super) fn install(
    requests: &[PackageRequest],
    held: &[PackageRequest],
    state: &str,
    obj: &Map<String, Value>,
    ctx: &ExecContext,
    system_state: &mut SystemState,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Installed, (bool, String)> {
    let latest = state == "latest";
    let plan = system_state
        .plan_packages(requests, latest, &ctx.environment)
        .map_err(|error| (false, error))?;
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
        let lifted_requests: Vec<&PackageRequest> = if bool_param(obj, "hold", false) {
            requests
                .iter()
                .filter(|request| {
                    held.iter()
                        .any(|entry| request.covered_by(entry, native.as_deref()))
                })
                .collect()
        } else {
            Vec::new()
        };
        let lifted: Vec<String> = lifted_requests
            .iter()
            .map(|request| request.selector())
            .collect();
        if !lifted.is_empty() {
            let mut marks = vec!["unhold"];
            marks.extend(lifted.iter().map(String::as_str));
            let (_, rc, stderr) = command(&marks, ctx).map_err(|error| (false, error))?;
            if rc != 0 {
                // `apt-mark unhold` can lift the valid selectors before a
                // bad one fails the command: the holds it released are
                // restored before the failure is reported, and a
                // restoration that fails is a known change beside it —
                // the host must never keep a silently dropped hold.
                let restored = restore_holds(&lifted_requests, native.as_deref(), command, ctx);
                let lost_hold = restored.is_err();
                return Err((
                    lost_hold,
                    beside(&format!("apt-mark unhold failed: {stderr}"), &restored),
                ));
            }
        }
        args.extend(plan.transaction.iter().map(String::as_str));
        // The transaction's result is captured, not propagated: the holds
        // must be restored whatever it answered — including an error that
        // never let apt-get start — before the error is reported.
        let install = command(&args, ctx);
        system_state.invalidate_packages();
        // The holds are restored whatever the transaction answered: the
        // host held the packages before the step, so the step leaves them
        // held — or reports the mark that could not be restored.
        let restored = restore_holds(&lifted_requests, native.as_deref(), command, ctx);
        // A hold that could not be restored is a known change: the
        // package stays unintentionally unheld. The error carries that
        // change beside both messages, because `modules::execute` turns
        // an Err into `changed: false` and would hide it.
        let lost_hold = restored.is_err();
        let (stdout, rc, stderr) =
            install.map_err(|error| (lost_hold, beside(&error, &restored)))?;
        if rc != 0 {
            // A failed apply-mode transaction may have changed the host
            // before it stopped — dpkg moved packages or left them
            // partially configured. The known change is reported from
            // the signals that say the host moved — the transition
            // touched the host's hold marks, a mark could not be
            // restored, or apt's summary counts moved packages — and not
            // from `lost_hold` alone: a refusal that moved nothing still
            // reports `changed: false`.
            let known_change = !lifted.is_empty() || lost_hold || summary_changed(&stdout);
            return Err((
                known_change,
                beside(&format!("apt install failed: {stderr}"), &restored),
            ));
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

/// The transaction's error beside the restoration that followed it, so a
/// run that failed names both: the failure, and the released holds that
/// were put back — or that could not be, a known change, because the
/// package stays unintentionally unheld.
fn beside(error: &str, restored: &Result<(), String>) -> String {
    match restored {
        Ok(()) => format!("{error}; the released holds were restored"),
        Err(mark) => format!("{error}; and the hold could not be restored: {mark}"),
    }
}

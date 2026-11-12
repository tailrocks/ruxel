//! The holds of an apt step: the marks `apt-mark` sets and the state the
//! host reports afterwards.
//!
//! A hold is only as good as the state the host reports, so the holds are
//! read before the marks run and read again afterwards, and the report
//! names the holds the host really has.

use super::{CommandResult, ExecContext};
use crate::system_state::{PackageRequest, parse_showhold};
use serde_json::{Value, json};

/// The outcome of the holds.
pub(super) enum Holds {
    /// The holds are in place: the flag says whether the host changed.
    Applied(bool),
    /// The marks ran, but the host does not hold every requested package.
    Unverified,
}

/// Holds the packages the requests name and reports the holds of the host.
/// The report names the holds the host really has, and a package the host
/// does not hold is a failure, not a success.
pub(super) fn hold_packages(
    holds: Vec<PackageRequest>,
    requests: &[PackageRequest],
    native: Option<&str>,
    ctx: &ExecContext,
    result: &mut Value,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<Holds, String> {
    let (marked, holds_after, failure) = apply_holds(holds, requests, native, ctx, command)?;
    result["holds"] = json!(
        holds_after
            .iter()
            .map(PackageRequest::selector)
            .collect::<Vec<_>>()
    );
    match failure {
        None => Ok(Holds::Applied(marked)),
        Some(message) => {
            // The marks ran, so the host may have changed; the report names
            // the packages the host does not hold.
            result["changed"] = json!(true);
            result["failed"] = json!(true);
            result["msg"] = json!(message);
            Ok(Holds::Unverified)
        }
    }
}

/// Holds every listed package the host does not hold yet, with `apt-mark
/// hold`, and reports the holds of the host afterwards. The holds are read
/// before anything runs, so a package that is already held is not held
/// again, and they are read again afterwards, so the report names the
/// holds the host really has.
fn apply_holds(
    held: Vec<PackageRequest>,
    requests: &[PackageRequest],
    native: Option<&str>,
    ctx: &ExecContext,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
) -> Result<(bool, Vec<PackageRequest>, Option<String>), String> {
    let to_hold: Vec<String> = requests
        .iter()
        .filter(|request| !held.iter().any(|entry| request.covered_by(entry, native)))
        .map(PackageRequest::selector)
        .collect();
    if to_hold.is_empty() {
        return Ok((false, held, None));
    }
    if ctx.check_mode {
        // A hold would be applied; in check mode nothing runs, and the
        // pending change is reported.
        return Ok((true, held, None));
    }
    let mut args = vec!["hold"];
    args.extend(to_hold.iter().map(String::as_str));
    let (_, rc, stderr) = command(&args, ctx)?;
    // A failing `apt-mark hold` can persist the valid selectors before a
    // bad one exits nonzero: the marks it took are a known change and the
    // holds it persisted are the step's declared state, so the report
    // reconciles them against the holds the host really has — the same
    // partial-update handling the unhold gets in `install.rs` — instead
    // of dropping them behind a bare error that claims `changed: false`.
    let failure = if rc == 0 {
        None
    } else {
        Some(format!("apt-mark hold failed: {stderr}"))
    };
    // The marks are only as good as the state the host reports: the holds
    // are read back, and a package the host does not list is not held. A
    // read-back that fails cannot unmake the mark the host already took:
    // the change is kept and the failure names the unverified holds.
    let after = match read_showhold(command, ctx) {
        Ok(after) => after,
        Err(error) => {
            return Ok((
                true,
                held,
                Some(match failure {
                    Some(failure) => {
                        format!("{failure}; the holds could not be read back: {error}")
                    }
                    None => format!(
                        "apt-mark hold applied but the holds could not be read back: {error}"
                    ),
                }),
            ));
        }
    };
    let not_held: Vec<String> = requests
        .iter()
        .filter(|request| !after.iter().any(|entry| request.covered_by(entry, native)))
        .map(PackageRequest::selector)
        .collect();
    let gaps = if not_held.is_empty() {
        None
    } else {
        Some(format!(
            "apt-mark hold did not take effect for: {}",
            not_held.join(", ")
        ))
    };
    let message = match (failure, gaps) {
        (None, None) => None,
        (Some(text), None) | (None, Some(text)) => Some(text),
        (Some(failure), Some(gaps)) => Some(format!("{failure}; {gaps}")),
    };
    Ok((true, after, message))
}

/// Puts back the holds the transaction lifted, so the host keeps the
/// state the step promised, and reads the marks back: an `apt-mark hold`
/// that exits 0 without taking effect would otherwise be reported as
/// verified, because the later reconciliation reads the snapshot taken
/// before the install.
pub(super) fn restore_holds(
    lifted_requests: &[&PackageRequest],
    native: Option<&str>,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
    ctx: &ExecContext,
) -> Result<(), String> {
    if lifted_requests.is_empty() {
        return Ok(());
    }
    let lifted: Vec<String> = lifted_requests
        .iter()
        .map(|request| request.selector())
        .collect();
    let mut marks = vec!["hold"];
    marks.extend(lifted.iter().map(String::as_str));
    let (_, rc, stderr) = command(&marks, ctx)?;
    if rc != 0 {
        return Err(format!("apt-mark hold failed: {stderr}"));
    }
    verify_holds(lifted_requests, native, command, ctx)
}

/// The restored holds are only as good as the state the host reports: the
/// holds are read back, and a lifted request the host does not list again
/// is a failure — the marks are the last thing the step promises, and
/// nothing after the restore re-reads them.
pub(super) fn verify_holds(
    requests: &[&PackageRequest],
    native: Option<&str>,
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
    ctx: &ExecContext,
) -> Result<(), String> {
    let after = read_showhold(command, ctx).map_err(|error| {
        format!("apt-mark hold applied but the holds could not be read back: {error}")
    })?;
    let not_held: Vec<String> = requests
        .iter()
        .filter(|request| !after.iter().any(|entry| request.covered_by(entry, native)))
        .map(|request| request.selector())
        .collect();
    if not_held.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "apt-mark hold did not take effect for: {}",
            not_held.join(", ")
        ))
    }
}

/// The packages the host holds, read with `apt-mark showhold`.
pub(super) fn read_showhold(
    command: &mut impl FnMut(&[&str], &ExecContext) -> CommandResult,
    ctx: &ExecContext,
) -> Result<Vec<PackageRequest>, String> {
    let (stdout, rc, stderr) = command(&["showhold"], ctx)?;
    if rc != 0 {
        return Err(format!("apt-mark showhold failed: {stderr}"));
    }
    parse_showhold(&stdout)
}

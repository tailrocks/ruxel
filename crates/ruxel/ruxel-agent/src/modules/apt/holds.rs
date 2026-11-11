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
    if rc != 0 {
        return Err(format!("apt-mark hold failed: {stderr}"));
    }
    // The marks are only as good as the state the host reports: the holds
    // are read back, and a package the host does not list is not held.
    let after = read_showhold(command, ctx)?;
    let not_held: Vec<String> = requests
        .iter()
        .filter(|request| !after.iter().any(|entry| request.covered_by(entry, native)))
        .map(PackageRequest::selector)
        .collect();
    if not_held.is_empty() {
        Ok((true, after, None))
    } else {
        Ok((
            true,
            after,
            Some(format!(
                "apt-mark hold did not take effect for: {}",
                not_held.join(", ")
            )),
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

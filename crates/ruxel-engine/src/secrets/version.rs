//! The fnox CLI version gate: `fnox set --from-file <path>` is the
//! byte-preserving store transport, and the flag exists only from fnox
//! v1.34.0.

use std::path::Path;
use std::process::{Command, Stdio};

/// Refuses the store when the installed fnox predates `fnox set
/// --from-file <path>`, which the fnox v1.34.0 release added. Without
/// the gate every store would fail on an unknown flag that names nothing
/// the operator can act on. A version the CLI does not report cannot be
/// held against it: the store then runs and its own failure is what the
/// run reports.
///
/// # Errors
/// Returns an error naming the required and the installed version.
pub(super) fn require_from_file(fnox: &Path) -> Result<(), String> {
    const MIN_FROM_FILE: (u64, u64, u64) = (1, 34, 0);
    let Some((major, minor, patch)) = fnox_version(fnox) else {
        return Ok(());
    };
    if (major, minor, patch) < MIN_FROM_FILE {
        return Err(format!(
            "fnox set --from-file requires fnox {}.{}.{} or newer; the installed fnox is \
             {major}.{minor}.{patch} and would fail the store with an unknown flag — \
             upgrade fnox before storing secrets",
            MIN_FROM_FILE.0, MIN_FROM_FILE.1, MIN_FROM_FILE.2
        ));
    }
    Ok(())
}

/// The version the fnox CLI reports for itself, if it reports one
/// parseable: `fnox --version`, its own report, never a guess.
fn fnox_version(fnox: &Path) -> Option<(u64, u64, u64)> {
    let output = Command::new(fnox)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reported = if stdout.trim().is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        stdout.into_owned()
    };
    parse_version(&reported)
}

/// The first `major.minor[.patch]` a `--version` report spells out.
/// Tokens that are not version numbers — the program name, build
/// metadata — parse as nothing and are skipped.
fn parse_version(report: &str) -> Option<(u64, u64, u64)> {
    report.split_whitespace().find_map(|token| {
        let mut parts = token.trim_start_matches('v').split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next().map_or(Some(0), |tail| {
            tail.split(['-', '+']).next().unwrap_or("").parse().ok()
        })?;
        Some((major, minor, patch))
    })
}

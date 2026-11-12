//! A declared login shell exists and is executable.

use std::os::unix::fs::PermissionsExt;

/// Whether the shell is an executable file. `useradd` and `usermod`
/// accept a missing or non-executable shell and only warn, which would
/// leave the account unable to log in while the apply reports success.
pub(super) fn shell_allowed(shell: &str) -> Result<(), String> {
    let meta = std::fs::metadata(shell)
        .map_err(|_| format!("user: shell {shell} is not an executable file"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("user: shell {shell} is not an executable file"));
    }
    Ok(())
}

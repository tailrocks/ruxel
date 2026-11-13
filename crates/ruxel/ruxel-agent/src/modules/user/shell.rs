//! A declared login shell exists and is executable by the account.

use std::os::unix::fs::PermissionsExt;

/// Whether the shell is executable by the account that signs in with
/// it. `useradd` and `usermod` accept a missing or non-executable shell
/// and only warn, which would leave the account unable to log in while
/// the apply reports success. Unix selects the permission class by the
/// caller's identity: an owner- or group-execute bit serves only the
/// file's owner or group, and the managed account runs as `other`
/// unless it owns the file — which this check cannot know before the
/// account exists — so the conservative class is the other-execute
/// bit, the same test the lowering passes a managed shell file
/// through (`executableMode`).
pub(super) fn shell_allowed(shell: &str) -> Result<(), String> {
    let meta = std::fs::metadata(shell)
        .map_err(|_| format!("user: shell {shell} is not an executable file"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o001 == 0 {
        return Err(format!(
            "user: shell {shell} is not executable for the account"
        ));
    }
    Ok(())
}

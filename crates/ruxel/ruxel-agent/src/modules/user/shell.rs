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
    ancestors_searchable(shell)
}

/// Every directory of the path the account traverses to reach the
/// shell must let it in: the file's own bits decide nothing above it,
/// and a shell the account cannot walk to — `/root/bin/zsh` at mode
/// `0755` beneath `/root` at `0700` — leaves login failing while the
/// root-run apply reports success. The account is not known before the
/// task runs (it may not exist yet), so the conservative class is the
/// one the file itself is judged by: the other bit, which every
/// account outside the file's own owner and group relies on. A lookup
/// error cannot follow a successful `metadata` on the file — the walk
/// crosses exactly the directories that lookup crossed — so only the
/// permission verdict is left to make here.
fn ancestors_searchable(shell: &str) -> Result<(), String> {
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(shell).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        let meta = std::fs::metadata(&prefix)
            .map_err(|error| format!("user: shell {shell}: {}: {error}", prefix.display()))?;
        if !meta.is_dir() || meta.permissions().mode() & 0o001 == 0 {
            return Err(format!(
                "user: shell {shell}: {} is not searchable for the account",
                prefix.display()
            ));
        }
    }
    Ok(())
}

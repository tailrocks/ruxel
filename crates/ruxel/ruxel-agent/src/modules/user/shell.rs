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

/// Every directory the account's lookup enters on its way to the
/// shell must let it in: the file's own bits decide nothing above it,
/// and a shell the account cannot walk to — `/root/bin/zsh` at mode
/// `0755` beneath `/root` at `0700` — leaves login failing while the
/// root-run apply reports success. The walk is the kernel's, not the
/// path's spelling: a symlink is followed wherever it stands, so the
/// directories its target chain traverses — absent from
/// `components(shell)` — are judged too. `/usr/local/bin/mysh` linking
/// to `/root/bin/mysh` walks `/root` and `/root/bin`, a chain of links
/// walks every step of it, and a link naming a directory of the path
/// walks its own target's directories before the path continues. The
/// account is not known before the task runs (it may not exist yet),
/// so the conservative class is the one the file itself is judged by:
/// the other bit, which every account outside the file's own owner and
/// group relies on. A lookup error cannot follow a successful
/// `metadata` on the file — the walk crosses exactly the directories
/// that lookup crossed — so only the permission verdict is left to
/// make here, and the links are followed within the `ELOOP` budget.
fn ancestors_searchable(shell: &str) -> Result<(), String> {
    // The resolutions the kernel still owes: the declared path first,
    // then each symlink's target with the rest of the lookup attached,
    // exactly as a lookup rewrites the walk.
    let mut pending = vec![std::path::PathBuf::from(shell)];
    let mut followed = 0;
    'resolution: while let Some(path) = pending.pop() {
        let components: Vec<std::path::Component<'_>> = path.components().collect();
        let mut dir = if path.is_absolute() {
            std::path::PathBuf::from("/")
        } else {
            std::path::PathBuf::new()
        };
        for (index, component) in components.iter().enumerate() {
            if *component == std::path::Component::RootDir {
                continue;
            }
            // The lookup of `component` happens in `dir`, which must
            // let the account in: the shell's parent as much as every
            // directory a traversed link's target names.
            searchable(&dir, shell)?;
            let entry = dir.join(component);
            let meta = std::fs::symlink_metadata(&entry)
                .map_err(|error| format!("user: shell {shell}: {}: {error}", entry.display()))?;
            if meta.file_type().is_symlink() {
                followed += 1;
                if followed > 40 {
                    return Err(format!(
                        "user: shell {shell}: {}: too many symbolic links",
                        entry.display()
                    ));
                }
                let target = std::fs::read_link(&entry).map_err(|error| {
                    format!("user: shell {shell}: {}: {error}", entry.display())
                })?;
                let mut rest = std::path::PathBuf::new();
                rest.extend(&components[index + 1..]);
                pending.push(if target.is_absolute() {
                    target.join(rest)
                } else {
                    dir.join(target).join(rest)
                });
                continue 'resolution;
            }
            if index + 1 < components.len() {
                if !meta.is_dir() {
                    return Err(format!(
                        "user: shell {shell}: {} is not searchable for the account",
                        entry.display()
                    ));
                }
                dir = entry;
            }
            // The last entry is the shell itself: `shell_allowed`
            // judged its bits by following the chain to it.
        }
    }
    Ok(())
}

/// Whether the account can search the directory a lookup enters: the
/// other-search bit, the same class the shell file is judged by. The
/// first lookup of a relative path enters the current directory.
fn searchable(dir: &std::path::Path, shell: &str) -> Result<(), String> {
    let probe = if dir.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        dir
    };
    let meta = std::fs::metadata(probe)
        .map_err(|error| format!("user: shell {shell}: {}: {error}", probe.display()))?;
    if !meta.is_dir() || meta.permissions().mode() & 0o001 == 0 {
        return Err(format!(
            "user: shell {shell}: {} is not searchable for the account",
            probe.display()
        ));
    }
    Ok(())
}

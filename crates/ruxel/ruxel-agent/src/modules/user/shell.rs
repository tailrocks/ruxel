//! A declared login shell exists and is executable by the account.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;

thread_local! {
    /// The shell paths an earlier File or Symlink step of the plan
    /// provides: check mode runs no file creation, so the live
    /// filesystem still lacks them when the account's task verifies
    /// the shell — the plan's own record stands in for the missing
    /// live entries the way `planned_group` stands in for a group
    /// `groupadd` has not created yet. The lowering orders every
    /// managed shell provider before the account
    /// (`linux-user-shell-symlink-chain-before-account.pkl`,
    /// `linux-user-shell-reconciled-later-stage.pkl`,
    /// `linux-user-shell-obsolete-provider-replaced.pkl`), and apply
    /// verifies the live shell the provider created, so the excuse
    /// holds in check mode only.
    static PLANNED_SHELL_PROVIDERS: RefCell<BTreeSet<String>> =
        const { RefCell::new(BTreeSet::new()) };
    /// The source each planned link declares: a managed link is only
    /// as good as its target chain, which check mode never creates,
    /// so the account's check-mode verification judges the effective
    /// chain — the planned links, then the planned file or the live
    /// filesystem — instead of the link's bare registration above. A
    /// link the plan never manages has no entry here: only the `file`
    /// state=link step writes this record.
    static PLANNED_SHELL_LINKS: RefCell<BTreeMap<String, String>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Records that a File or Symlink step of the plan manages `path`:
/// the account's check-mode verification reads it instead of failing
/// on the live filesystem the step has not mutated yet. The File and
/// Symlink steps are the producers of this record: every executed
/// `copy` and `file` state=link task notes what it manages, in check
/// mode as much as in apply — the lowering orders every managed
/// shell provider before the account, and apply verifies the live
/// shell the provider created, so the record excuses nothing there.
pub(crate) fn note_planned_shell_provider(path: &str) {
    PLANNED_SHELL_PROVIDERS.with(|providers| {
        providers.borrow_mut().insert(path.to_string());
    });
}

/// Records that a `file` state=link step of the plan manages `path`
/// as a link to `src`: the account's check-mode verification follows
/// this declared chain instead of trusting the link's bare existence.
/// The link registers as a provider too, so the registry still names
/// the destination the plan manages.
pub(crate) fn note_planned_shell_link(path: &str, src: &str) {
    PLANNED_SHELL_LINKS.with(|links| {
        links.borrow_mut().insert(path.to_string(), src.to_string());
    });
    note_planned_shell_provider(path);
}

/// Forgets that a step of the plan manages `path` as a shell: a `copy`
/// or `file` task that leaves a non-executable file behind supersedes
/// the executable provider the registry still names, the way the
/// lowering refuses a managed shell file without an executable mode —
/// the account's task reads the replacement, not the superseded
/// provider. A superseded link takes its declared chain with it, so a
/// later lookup cannot follow a chain the plan no longer manages.
pub(crate) fn forget_planned_shell_provider(path: &str) {
    PLANNED_SHELL_PROVIDERS.with(|providers| {
        providers.borrow_mut().remove(path);
    });
    PLANNED_SHELL_LINKS.with(|links| {
        links.borrow_mut().remove(path);
    });
}

/// The shell-provider record of an executed file-provisioning task:
/// an executable mode registers the path the task leaves behind, a
/// declared non-executable mode unregisters it — the task supersedes
/// whatever the registry names — and no declared mode leaves the
/// record alone, since the task preserves whatever is there. Only the
/// other-execute bit admits: the account the shell serves runs as
/// `other` unless it owns the file, the class `shell_allowed`
/// judges by.
pub(crate) fn note_managed_shell_file(path: &str, mode: Option<u32>) {
    match mode {
        Some(bits) if bits & 0o001 != 0 => note_planned_shell_provider(path),
        Some(_) => forget_planned_shell_provider(path),
        None => {}
    }
}

/// Whether an earlier step of the plan provides the shell `path`.
/// Test-only: production judges the effective target chain (see
/// `planned_shell_chain_ok`), not bare registration.
#[cfg(test)]
pub(super) fn planned_shell_provided(path: &str) -> bool {
    PLANNED_SHELL_PROVIDERS.with(|providers| providers.borrow().contains(path))
}

/// Clears the plan's shell-provider record: a new plan starts with no
/// provider, the way `plan::reset` clears the identity record.
pub(super) fn reset_planned_shell_providers() {
    PLANNED_SHELL_PROVIDERS.with(|providers| {
        providers.borrow_mut().clear();
    });
    PLANNED_SHELL_LINKS.with(|links| {
        links.borrow_mut().clear();
    });
}

/// Whether the plan's declared providers make `shell` executable: the
/// chain of planned links from the shell, resolved the way the
/// account's lookup resolves them — a relative source against the
/// link's own directory — down to a terminal the plan provides as a
/// file, or the live filesystem where the plan provides nothing
/// further. A dangling or non-executable terminal fails, a cycle or a
/// chain past the lookup budget fails, and a link the plan never
/// managed is not followed: only `note_planned_shell_link` declares.
fn planned_shell_chain_ok(shell: &str) -> bool {
    let mut current = std::path::PathBuf::from(shell);
    for _ in 0..40 {
        let key = current.to_string_lossy().into_owned();
        let declared = PLANNED_SHELL_LINKS.with(|links| links.borrow().get(&key).cloned());
        let Some(src) = declared else {
            return PLANNED_SHELL_PROVIDERS.with(|providers| providers.borrow().contains(&key))
                || shell_allowed(&key).is_ok();
        };
        let target = std::path::PathBuf::from(src);
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .map_or(target.clone(), |dir| dir.join(target))
        };
    }
    false
}

/// Whether the account may declare `shell` as its login shell: the
/// live `shell_allowed` check, except that check mode excuses a shell
/// an earlier File or Symlink step of the plan provides — check mode
/// runs no file creation, so the live filesystem still lacks it,
/// while apply runs the provider first and verifies the live shell
/// it created. A shell no step provides is refused here the way
/// apply refuses it: the dry run must not report a change that cannot
/// succeed.
pub(super) fn shell_provided(shell: &str, check_mode: bool) -> Result<(), String> {
    match shell_allowed(shell) {
        Ok(()) => Ok(()),
        Err(error) => {
            if check_mode && planned_shell_chain_ok(shell) {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

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

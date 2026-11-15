use rustix::fs::{Mode, fchmod};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// The declared home path is usable before the account is touched:
/// `usermod -d` moves the passwd field even when the path is a regular
/// file, so the path is validated before any mutation — a refused apply
/// must leave the account as it was. A symlink is refused whatever it
/// targets: a home in an attacker-writable parent must never be
/// followed to a directory the account does not own, and every ancestor
/// of the path — which the lookup and the later `create_dir_all` and
/// `lchown` all traverse — is inspected the same way. An existing
/// directory is adopted only where [`adoption_trusted`] admits it: a
/// directory a non-root principal may have placed must never become the
/// account's home with its contents. Only a missing path under trusted
/// ancestors is usable — the repair creates the home beneath them, so
/// a creation beneath a directory a non-root principal can write would
/// hand the fresh home to that principal's rename-and-plant: any other
/// lookup error — an ancestor that is a
/// regular file (`NotADirectory`), a permission refusal — propagates
/// before any mutation, for `usermod -d` would otherwise move the passwd
/// field first and leave a partially applied account behind.
pub(super) fn home_usable(home: &str) -> Result<(), String> {
    ancestors_usable(home)?;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => {
            adoption_trusted(home, &meta)?;
            Ok(())
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => trusted_ancestors(home),
        Err(error) => Err(format!("user: home {home}: {error}")),
    }
}

/// Every ancestor of the home path, inspected without following it:
/// `create_dir_all` and `lchown` are path-based, so a symlinked
/// component would be traversed as root and the operation could create
/// or chown beneath a target the account does not own. A symlinked
/// ancestor is refused wherever a non-root principal could plant or
/// replace it, and admitted only where the parent is root's and
/// unwritable to the rest — the system links an operator places (`/var`
/// on macOS) are the operator's configuration, not the attacker's to
/// redirect. The walk judges the shape, not the trust: an existing
/// directory of any owner passes here — the trust of an adopted home is
/// [`adoption_trusted`]'s to judge — while a missing leaf is created
/// beneath its ancestors, so the creation's entry points additionally
/// require [`trusted_ancestors`]. A missing component ends the walk: the
/// rest of the path does not exist yet, so nothing can traverse it.
pub(super) fn ancestors_usable(home: &str) -> Result<(), String> {
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(home).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => planted_in(&prefix, home)?,
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "user: home {home}: {} is not a directory",
                    prefix.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(())
}

/// Whether a symlinked ancestor of the home path stands where a
/// non-root principal could plant or replace it: its parent is group-
/// or other-writable, or not root's. Such a link is the attacker's to
/// redirect a root traversal through; a link only root can replace is
/// system configuration the operator chose, and the path resolves
/// through it as it always did — but only through directories the
/// target leads through that are trusted the same way, so a link in a
/// trusted directory cannot point at a home a non-root principal
/// pre-created (`/srv/homes -> /tmp/homes`, `/home -> /tmp/attacker`).
fn planted_in(link: &Path, home: &str) -> Result<(), String> {
    let parent = standing_parent(link, home)?;
    let target = std::fs::read_link(link)
        .map_err(|error| format!("user: home {home}: {}: {error}", link.display()))?;
    symlink_target_trusted(&parent, &target, home)
}

/// The directory a symlinked ancestor stands in, trusted the way the
/// symlinked-ancestor rule trusts a parent: root's and writable by no
/// one else — the operator's configuration, never the attacker's to
/// redirect.
fn standing_parent(link: &Path, home: &str) -> Result<PathBuf, String> {
    let parent = match link.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let meta = std::fs::metadata(&parent)
        .map_err(|error| format!("user: home {home}: {}: {error}", parent.display()))?;
    if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        return Err(format!(
            "user: home {home}: {} is a symlink in {}, which a non-root principal can rewrite",
            link.display(),
            parent.display()
        ));
    }
    Ok(parent)
}

/// Every directory a symlink target leads through, trusted the way
/// trusted ancestors are: the resolution starts where the link stands —
/// a relative target against its parent directory, the way the kernel
/// resolves it — and every existing directory on the way must be the
/// operator's own and writable by no one else. There is no carve-out
/// for group writability: a group-writable intermediate lets any member
/// of the group rename the resolved target aside after the check and
/// plant a tree the managed account then executes at login — and the
/// lexical walk already refuses a group-writable ancestor, so a chain
/// check that admitted one would be inconsistent with it. A nested link
/// is admitted where it stands the same way, within the expansion
/// budget; a missing component ends the walk, for the rest does not
/// exist yet and nothing could have been planted under it.
pub(super) fn symlink_target_trusted(base: &Path, target: &Path, home: &str) -> Result<(), String> {
    target_dirs_trusted(&join_normalized(base, target), home, 8)
}

/// A symlink target joined onto the directory the link stands in, with
/// `.` and `..` resolved lexically: the kernel resolves the target the
/// same way before the walk opens any of it, so the validation sees the
/// directories the operation will enter.
fn join_normalized(base: &Path, target: &Path) -> PathBuf {
    let mut resolved = if target.is_absolute() {
        PathBuf::from("/")
    } else {
        base.to_path_buf()
    };
    for part in target.components() {
        match part {
            Component::Normal(name) => resolved.push(name),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    resolved
}

/// Every existing directory of a resolved symlink target, trusted the
/// way trusted ancestors are. A nested link is admitted where it stands
/// and the walk continues under its own resolved target, within the
/// expansion budget — a loop of links ends the walk instead of the
/// operation.
fn target_dirs_trusted(resolved: &Path, home: &str, budget: u8) -> Result<(), String> {
    let mut prefix = PathBuf::new();
    for component in resolved.components() {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => {
                if budget == 0 {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} leads through too many symlinks",
                        prefix.display()
                    ));
                }
                let parent = standing_parent(&prefix, home)?;
                let nested = std::fs::read_link(&prefix)
                    .map_err(|error| format!("user: home {home}: {}: {error}", prefix.display()))?;
                let mut spliced = join_normalized(&parent, &nested);
                let rest = resolved
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|_| Path::new(""))
                    .to_path_buf();
                for part in rest.components() {
                    spliced.push(part);
                }
                return target_dirs_trusted(&spliced, home, budget - 1);
            }
            Ok(meta) if meta.is_dir() => {
                // Every directory on the chain — the resolved target
                // and every intermediate alike — takes the full ancestor
                // rule (see `symlink_target_trusted`): a principal that
                // can write any of them can redirect the home.
                if !own_directory(meta.uid(), meta.mode()) {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} is writable by others or not the operator's, so a non-root principal may have placed the home beneath it",
                        prefix.display()
                    ));
                }
            }
            Ok(_) => {
                return Err(format!(
                    "user: home {home}: {} is not a directory",
                    prefix.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(())
}

/// The mode a created directory is given: the mode the creation
/// asked for is only a request — the process's umask takes bits away
/// from it, and a default ACL on the parent grants the ones the request
/// does not ask for — so the mode is enforced on the directory itself,
/// after it is created. The walk creates parents traversable and never
/// world-writable, and the home takes the policy mode: the trust the
/// adoption checks rely on.
pub(super) fn enforce_mode(fd: &OwnedFd, mode: Mode, home: &str) -> Result<(), String> {
    fchmod(fd, mode).map_err(|error| format!("user: create home {home}: {error}"))
}

/// Whether the directory is the operator's own and writable by no one
/// else: only the operator could have created it and only the operator
/// could have filled it, so its adoption hands the account no
/// principal's contents but the operator's. Every directory of a
/// resolved symlink target takes this same rule — the lexical ancestors
/// and the chain intermediaries alike, with no carve-out for group
/// writability (see `symlink_target_trusted`).
pub(super) fn own_directory(uid: u32, mode: u32) -> bool {
    (uid == 0 || uid == operating_uid()) && mode & 0o022 == 0
}

/// The uid the agent operates as: the owner of a file the agent just
/// created. The agent runs as root, and the operator's staging is the
/// operator's own — but reading the euid directly needs an unsafe call
/// the workspace forbids, so the uid is read off a probe file the
/// process creates itself: the kernel owns new files to their creator.
/// The probe's name is unique to the process and created atomically
/// (`create_new`), so a planted file never stands in for it — a name
/// something else holds only fails the creation and the next name is
/// tried. Where no probe can be created the answer matches nothing:
/// only root-owned directories are trusted then.
pub(super) fn operating_uid() -> u32 {
    static OPERATOR: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *OPERATOR.get_or_init(|| probe_operating_uid().unwrap_or(u32::MAX))
}

fn probe_operating_uid() -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt;
    let dir = std::env::temp_dir();
    for attempt in 0..100 {
        let probe = dir.join(format!("ruxel-operator-{}-{attempt}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => {
                let uid = file.metadata().map(|meta| meta.uid());
                drop(file);
                let _ = std::fs::remove_file(&probe);
                return uid.map_err(|error| error.to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("the operator probe could not be created".to_string())
}

/// An existing home directory the account adopts: the directory is
/// adopted only where it is the operator's own or stands where only
/// the operator could have placed it. A directory under a path a
/// non-root principal can write (`/tmp/alice`) may have been
/// pre-created by that principal with contents the new account would
/// then execute at login — `useradd -m` reuses the directory and the
/// reconciliation only changes its ownership, preserving whatever the
/// directory holds. Such an adoption is refused; the home is created
/// fresh instead where it is missing (`mkdir` fails where a directory
/// is already there, so a raced pre-creation is refused the same
/// way). The operator's own staging — a leftover `/home/alice`, a
/// same-stage file's directory — is adopted as before.
pub(super) fn adoption_trusted(home: &str, leaf: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    // The operator's own directory is adopted wherever it stands: no
    // other principal could have created or filled it.
    if own_directory(leaf.uid(), leaf.mode()) {
        return Ok(());
    }
    // Anything else is adopted only under trusted ancestors — and only
    // where it is itself writable by no one else, or anyone's contents
    // in it would become the account's.
    if leaf.mode() & 0o022 != 0 {
        return Err(format!(
            "user: refusing to adopt the existing home {home}: the directory is writable by others, so its contents may be a non-root principal's"
        ));
    }
    trusted_ancestors(home)
}

/// Every existing ancestor of the home path, trusted the way the
/// symlinked-ancestor rule trusts a parent: root's — or the
/// operator's own — and writable by no one else. A symlinked ancestor
/// resolves through its whole target chain the same way: a link only
/// root can replace is admitted, and every directory the target leads
/// through must be trusted too. A directory a
/// non-root principal can write is a directory that principal can
/// plant the home under, so the adoption is refused there. A missing
/// component ends the walk: the rest of the path does not exist yet,
/// so nothing could have been planted under it.
pub(super) fn trusted_ancestors(home: &str) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(home).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => planted_in(&prefix, home)?,
            Ok(meta) if meta.is_dir() => {
                if !own_directory(meta.uid(), meta.mode()) {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} is writable by others or not the operator's, so a non-root principal may have placed the home",
                        prefix.display()
                    ));
                }
            }
            Ok(_) => {
                return Err(format!(
                    "user: home {home}: {} is not a directory",
                    prefix.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(())
}

/// The adoption the opened descriptors pin: the directory the walk
/// opened, re-verified after the open. The path the pre-open check
/// inspected may have been replaced in between — a directory renamed
/// aside where a writable ancestor allows it — so the trust decision
/// is re-made on the inode the operation will own, never on the path:
/// the operator's own directory is adopted, anything else only under
/// trusted ancestors. A replacement the operator could not have made
/// is refused; the descriptors name the replacement, so the refusal
/// sees it.
pub(super) fn adoption_pinned(home: &str, fd: &OwnedFd) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let duplicate = fd
        .try_clone()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    let meta = std::fs::File::from(duplicate)
        .metadata()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    if own_directory(meta.uid(), meta.mode()) {
        return Ok(());
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!(
            "user: refusing to adopt the existing home {home}: the directory is writable by others, so its contents may be a non-root principal's"
        ));
    }
    trusted_ancestors(home)
}

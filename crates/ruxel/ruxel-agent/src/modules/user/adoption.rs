use rustix::fs::{Mode, fchmod};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// The declared home path is usable before the account is touched:
/// `usermod -d` moves the passwd field even when the path is a regular
/// file, so the path is validated before any mutation. A symlink is
/// refused whatever it targets, and every ancestor is inspected the
/// same way. An existing directory is adopted only where
/// [`adoption_trusted`] admits it for the managed account; a missing
/// path is usable only under trusted ancestors.
pub(super) fn home_usable(home: &str, managed: &ManagedAccount<'_>) -> Result<(), String> {
    ancestors_usable(home)?;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => {
            adoption_trusted(home, &meta, managed)?;
            Ok(())
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => trusted_ancestors(home),
        Err(error) => Err(format!("user: home {home}: {error}")),
    }
}

/// Every ancestor of the home path, inspected without following it:
/// path-based creation and ownership would traverse a symlinked
/// component as root. A symlinked ancestor is refused wherever a
/// non-root principal could plant or replace it, and admitted only
/// where the parent is root's and closed. A missing component ends the
/// walk: the rest does not exist yet.
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

/// Whether a symlinked ancestor stands where a non-root principal
/// could plant or replace it: a link only root can replace is system
/// configuration the operator chose, and the path resolves through it
/// as it always did — but only through directories the target leads
/// through that are trusted the same way, so a link in a trusted
/// directory cannot point at a home a non-root principal pre-created
/// (`/srv/homes -> /tmp/homes`, `/home -> /tmp/attacker`).
fn planted_in(link: &Path, home: &str) -> Result<(), String> {
    let parent = standing_parent(link, home)?;
    let target = std::fs::read_link(link)
        .map_err(|error| format!("user: home {home}: {}: {error}", link.display()))?;
    symlink_target_trusted(&parent, &target, home)
}

/// The directory a symlinked ancestor stands in: root's and writable
/// by no one else — the operator's configuration.
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
/// trusted ancestors are — except that an intermediate admits a
/// root-owned directory whose group writability belongs to a
/// privileged group, whose every writer is already privileged. The
/// resolved target itself always takes the full ancestor rule. A nested
/// link is admitted where it stands the same way, within the expansion
/// budget; a missing component ends the walk, for nothing could have
/// been planted under it.
pub(super) fn symlink_target_trusted(base: &Path, target: &Path, home: &str) -> Result<(), String> {
    target_dirs_trusted(&join_normalized(base, target), home, 8)
}

/// A symlink target joined onto the directory the link stands in,
/// with `.` and `..` resolved lexically the way the kernel resolves
/// the target before the walk opens any of it.
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
                // The resolved target takes the full ancestor rule; an
                // intermediate takes the chain rule: a principal that
                // can write the target can redirect the home, while an
                // intermediate admits root-owned group writability by a
                // privileged group.
                let trusted = if prefix == *resolved {
                    own_directory(meta.uid(), meta.mode())
                } else {
                    chain_dir_trusted(meta.uid(), meta.gid(), meta.mode())
                };
                if !trusted {
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

/// The mode a created directory is given, enforced on the directory
/// itself after it is created: the requested mode is only a request —
/// the process's umask takes bits away, and a default ACL on the
/// parent grants the ones the request does not ask for. Parents are
/// created traversable and never world-writable, and the home takes
/// the policy mode: the trust the adoption checks rely on.
pub(super) fn enforce_mode(fd: &OwnedFd, mode: Mode, home: &str) -> Result<(), String> {
    fchmod(fd, mode).map_err(|error| format!("user: create home {home}: {error}"))
}

/// Whether the directory is the operator's own and writable by no one
/// else — so its adoption hands the account no principal's contents
/// but the operator's — or a chain intermediate under the exception.
pub(super) fn own_directory(uid: u32, mode: u32) -> bool {
    (uid == 0 || uid == operating_uid()) && mode & 0o022 == 0
}

/// Whether a chain intermediate is trusted: the full ancestor rule, or
/// a root-owned directory whose group writability belongs to a
/// privileged group — the owner is root, the group holds no
/// unprivileged principal, and others cannot write, so no unprivileged
/// principal can plant through it. The exception is off-Linux only:
/// on Linux a lower-privileged member of the owning group can rename
/// the target aside and plant a replacement home, so even `root:root
/// 0775` refuses there. The other-triplet check is other-write, not
/// other-closed: others traverse every root-owned system directory
/// already (including `/`), and traversal without write cannot rename
/// anything aside. An other-closed check would refuse `root:admin
/// 0775` layouts — macOS routes all of `/home` through one — without
/// buying any privilege boundary.
pub(super) fn chain_dir_trusted(uid: u32, gid: u32, mode: u32) -> bool {
    chain_dir_trusted_on(uid, gid, mode, cfg!(target_os = "linux"))
}

/// The chain rule for the platform `strict` names: the Linux rule
/// trusts the strict base alone, while other platforms keep the
/// privileged-group exception. Split out so the Linux refusal is
/// provable on any host.
pub(super) fn chain_dir_trusted_on(uid: u32, gid: u32, mode: u32, strict: bool) -> bool {
    own_directory(uid, mode)
        || (!strict
            && uid == 0
            && mode & 0o002 == 0
            && mode & 0o020 != 0
            && privileged_gid(gid))
}

/// Whether the group holds no unprivileged principal: root everywhere,
/// and admin on macOS, whose members can already sudo.
pub(super) fn privileged_gid(gid: u32) -> bool {
    gid == 0 || (cfg!(target_os = "macos") && gid == 80)
}

/// The uid the agent operates as, read off a probe file the process
/// creates itself: the kernel owns new files to their creator, and the
/// workspace forbids the unsafe call that reads the euid directly. The
/// probe's name is unique to the process and created atomically
/// (`create_new`), so a planted file never stands in for it. Where no
/// probe can be created the answer matches nothing: only root-owned
/// directories are trusted then.
pub(super) fn operating_uid() -> u32 {
    static OPERATOR: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *OPERATOR.get_or_init(|| probe_operating_uid().unwrap_or(u32::MAX))
}

fn probe_operating_uid() -> Result<u32, String> {
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

/// The account a home operation manages: the adoption admits an
/// existing directory only where the passwd home field names this
/// account — name and number both. The number is `None` where the
/// step states none and the allocator draws it.
pub(super) struct ManagedAccount<'a> {
    pub(super) name: &'a str,
    pub(super) uid: Option<u32>,
}

#[cfg(test)]
impl<'a> ManagedAccount<'a> {
    /// The managed account where the plan states no number.
    pub(super) fn unnumbered(name: &'a str) -> Self {
        ManagedAccount { name, uid: None }
    }
}

/// Whether the passwd claim hands the leaf to the managed account —
/// name and number both — with the leaf carrying the claimant's
/// number, so the entry read is the live one, not a stale line.
pub(super) fn managed_claim(
    claimant: &str,
    uid: u32,
    leaf_uid: u32,
    managed: &ManagedAccount<'_>,
) -> bool {
    claimant == managed.name && managed.uid == Some(uid) && uid == leaf_uid
}

/// An existing home directory the account adopts, only under trusted
/// ancestors — where it is root's or the operator's own, or the
/// managed account's whose passwd home field names the path. A
/// directory another principal owns may have been pre-created by that
/// principal with contents the new account would then execute at login,
/// so it is refused whatever its mode: a 0700 leaf under trusted
/// ancestors is still another principal's. The ancestors weigh even
/// the operator's own staging — a leftover `/home/alice`, a
/// same-stage file's directory — since a renamed-aside leaf replanted
/// under a writable ancestor must never become the account's. A
/// creation whose account is not in the database yet misses the
/// intended lookup and still takes the operator's rule, so the miss
/// never fails a valid creation.
pub(super) fn adoption_trusted(
    home: &str,
    leaf: &std::fs::Metadata,
    managed: &ManagedAccount<'_>,
) -> Result<(), String> {
    // The operator's own directory is adopted under trusted ancestors
    // too: only the ancestors weigh a leaf renamed aside and replanted.
    if own_directory(leaf.uid(), leaf.mode()) {
        return trusted_ancestors(home);
    }
    // The managed account's own leaf is adopted under trusted
    // ancestors — and only where it is itself writable by no one
    // else, or anyone's contents in it would become the account's.
    // The claim must name the managed account itself, so adopting
    // `/home/bob` for Alice is refused while Bob still points there.
    if let Some((claimant, uid)) = intended_claim(home)
        && managed_claim(&claimant, uid, leaf.uid(), managed)
    {
        if leaf.mode() & 0o022 != 0 {
            return Err(format!(
                "user: refusing to adopt the existing home {home}: the directory is writable by others, so its contents may be a non-root principal's"
            ));
        }
        return trusted_ancestors(home);
    }
    Err(format!(
        "user: refusing to adopt the existing home {home}: the directory belongs to another principal, so its contents may be theirs, not the account's"
    ))
}

/// The account the passwd database intends the home for: the name and
/// number from the single line whose home field names the path — or
/// `None` where none names it, or where two do.
pub(super) fn intended_claim(home: &str) -> Option<(String, u32)> {
    sole_claimant(&std::fs::read_to_string("/etc/passwd").ok()?, home)
}

/// The sole claimant in the passwd text, voided by a second claimant.
pub(super) fn sole_claimant(passwd: &str, home: &str) -> Option<(String, u32)> {
    let mut found: Option<(String, u32)> = None;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.get(5) != Some(&home) {
            continue;
        }
        let uid: u32 = fields.get(2)?.parse().ok()?;
        if found.replace((fields.first()?.to_string(), uid)).is_some() {
            return None;
        }
    }
    found
}

/// Every existing ancestor of the home path, trusted the way the
/// symlinked-ancestor rule trusts a parent: root's — or the
/// operator's own — and writable by no one else. A directory a
/// non-root principal can write is one that principal can plant the
/// home under, so the adoption is refused there. A missing component
/// ends the walk: the rest does not exist yet.
pub(super) fn trusted_ancestors(home: &str) -> Result<(), String> {
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

/// The adoption the opened descriptors pin, re-verified after the
/// open: the path the pre-open check inspected may have been replaced
/// in between, so the trust decision is re-made on the inode the
/// operation will own, never on the path.
pub(super) fn adoption_pinned(
    home: &str,
    fd: &OwnedFd,
    managed: &ManagedAccount<'_>,
) -> Result<(), String> {
    let duplicate = fd
        .try_clone()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    let meta = std::fs::File::from(duplicate)
        .metadata()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    adoption_trusted(home, &meta, managed)
}

//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use rustix::fs::{
    AtFlags, Gid, Mode, OFlags, Uid, fchmod, fchown, fstat, mkdirat, open, openat, readlinkat,
    statat,
};
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

/// The mode a created home directory gets: the account's own, not the
/// world's to read or write. The schema exposes no home-mode field, so
/// the mode is the agent's policy — the conventional mode of a home
/// directory (`useradd -D`'s `HOME_MODE`).
const HOME_MODE: Mode = Mode::from_raw_mode(0o750);

/// The mode a created parent of the home gets: the path to the home is
/// traversable, and never world-writable — a parent that others may
/// write is a parent they may replace.
const PARENT_MODE: Mode = Mode::from_raw_mode(0o755);

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
/// account's home with its contents. Only a missing
/// path is usable: any other lookup error — an ancestor that is a
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
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
/// redirect. A missing component ends the walk: the rest of the path
/// does not exist yet, so nothing can traverse it.
fn ancestors_usable(home: &str) -> Result<(), String> {
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
/// through it as it always did.
fn planted_in(link: &std::path::Path, home: &str) -> Result<(), String> {
    let parent = match link.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let meta = std::fs::metadata(parent)
        .map_err(|error| format!("user: home {home}: {}: {error}", parent.display()))?;
    if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        return Err(format!(
            "user: home {home}: {} is a symlink in {}, which a non-root principal can rewrite",
            link.display(),
            parent.display()
        ));
    }
    Ok(())
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
fn operating_uid() -> u32 {
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

/// Whether the directory is the operator's own and writable by no one
/// else: only the operator could have created it and only the operator
/// could have filled it, so its adoption hands the account no
/// principal's contents but the operator's.
fn own_directory(uid: u32, mode: u32) -> bool {
    (uid == 0 || uid == operating_uid()) && mode & 0o022 == 0
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
fn adoption_trusted(home: &str, leaf: &std::fs::Metadata) -> Result<(), String> {
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
/// operator's own — and writable by no one else. A directory a
/// non-root principal can write is a directory that principal can
/// plant the home under, so the adoption is refused there. A missing
/// component ends the walk: the rest of the path does not exist yet,
/// so nothing could have been planted under it.
fn trusted_ancestors(home: &str) -> Result<(), String> {
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
fn adoption_pinned(home: &str, fd: &OwnedFd) -> Result<(), String> {
    let meta = fstat(fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if own_directory(meta.st_uid, meta.st_mode.into()) {
        return Ok(());
    }
    if meta.st_mode & 0o022 != 0 {
        return Err(format!(
            "user: refusing to adopt the existing home {home}: the directory is writable by others, so its contents may be a non-root principal's"
        ));
    }
    trusted_ancestors(home)
}

/// The home `useradd` creates for an account: the declared one, or the
/// `/home/<name>` the lowering's footprint reserves. The base directory
/// is host configuration (`useradd -D`), so the creation passes the home
/// explicitly: `useradd -m` without `-d` would put it wherever the
/// host's configured base points, and a same-stage file at the reserved
/// path would collide with account creation.
pub(super) fn claimed_home(home: Option<&str>, name: &str) -> String {
    home.map_or_else(|| format!("/home/{name}"), str::to_string)
}

/// The gid the home directory is created with: the requested primary
/// group's, whose `usermod -g` has already moved the account by the time
/// the directory is created — the account's old gid would own it and no
/// later apply would repair it — or the account's current one when the
/// step names no group. `None` is check mode's answer for a group that
/// is not on the host yet (see `planned_gid`): the plan must not require
/// the new group's live gid, and the directory's final owner is settled
/// by the primary-group change the plan already reports.
pub(super) fn home_gid(
    group: Option<&str>,
    current_gid: u64,
    check_mode: bool,
) -> Result<Option<u64>, String> {
    match group {
        None => Ok(Some(current_gid)),
        Some(g) => Ok(super::planned_gid(g, check_mode)?.map(u64::from)),
    }
}

/// The declared home directory exists and belongs to the account:
/// `usermod -d` moves the passwd field only, so the directory is the
/// agent's to create — and one that exists but drifted to another uid or
/// gid is the agent's to reconcile, never a no-change early return. A
/// symlink at the path is refused, every ancestor is inspected before
/// the path is traversed, and ownership changes never follow a link
/// (`lchown`), so neither a link at the path nor one above it can ever
/// have a target handed to the account. The mutation itself runs on
/// file descriptors the walk opened without following a link: a
/// directory renamed aside after `ancestors_usable` inspected it leaves
/// the operation on the original inode, never on a replacement a
/// non-root principal planted. Returns whether it changed (or would
/// change in check mode).
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
) -> Result<bool, String> {
    ancestors_usable(home)?;
    let dir = Path::new(home);
    let uid =
        u32::try_from(uid).map_err(|_| format!("user: home {home}: uid {uid} does not fit"))?;
    let gid =
        u32::try_from(gid).map_err(|_| format!("user: home {home}: gid {gid} does not fit"))?;
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => {
            // An existing directory is adopted, never taken blindly:
            // a directory a non-root principal may have placed must
            // never become the account's home with its contents.
            adoption_trusted(home, &meta)?;
            if meta.uid() == uid && meta.gid() == gid {
                return Ok(false);
            }
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, false)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the path the check above inspected may have been
            // replaced in between.
            adoption_pinned(home, &fd)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, true)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the leaf the lookup found missing may have been
            // created in between — a directory a non-root principal
            // planted must never become the account's home with its
            // contents. A directory the creation just made is the
            // operator's own and passes.
            adoption_pinned(home, &fd)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

/// The home's repair a later failure takes back: the ownership the
/// repair found, restored — or the directory the repair created,
/// removed. The reversal runs on the descriptors the safe walk opens,
/// never through a link a concurrent rewrite plants, for the repair's
/// own reason. Removing the created directory takes back exactly what
/// the repair added: the removal refuses a directory that is not
/// empty, so contents a concurrent writer planted are kept and
/// reported rather than deleted.
pub(super) fn rollback_home(path: &str, prior: Option<(u64, u64)>) -> Result<(), String> {
    match prior {
        Some((uid, gid)) => {
            let uid = u32::try_from(uid)
                .map_err(|_| format!("user: take back home {path}: uid {uid} does not fit"))?;
            let gid = u32::try_from(gid)
                .map_err(|_| format!("user: take back home {path}: gid {gid} does not fit"))?;
            own_through(&open_home(path, false)?, uid, gid, path)
        }
        None => std::fs::remove_dir(path)
            .map_err(|error| format!("user: take back created home {path}: {error}")),
    }
}

/// The claimed home of the account the step created: the directory is
/// reconciled with the account's final identity, and the account is
/// taken back when the reconciliation fails. A home that cannot take
/// the ownership — an immutable directory, a directory on a read-only
/// mount — fails the reconciliation, and the account the step created
/// must not stay behind in the passwd database when the task fails:
/// the host's account database is the workflow's to keep, and a
/// refused task leaves it as it was.
pub(super) fn reconcile_created_home(
    name: &str,
    home: &str,
    created: &super::PasswdEntry,
    check_mode: bool,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    match ensure_home(home, created.uid, created.gid, check_mode) {
        Ok(changed) => Ok(changed),
        Err(error) => match remove_account(name) {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!(
                "{error}; the account {name} could not be taken back: {rollback}"
            )),
        },
    }
}

/// The home directory opened through its parent: the walk opens the
/// directories of the path without following a link, and the leaf is
/// created or opened as itself — never resolved through a link a
/// concurrent rewrite plants at it, so the ownership below lands on
/// the directory the path named, or on nothing at all.
fn open_home(home: &str, create: bool) -> Result<OwnedFd, String> {
    let path = Path::new(home);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| format!("user: home {home}: no directory name"))?;
    let dir = open_chain(parent, home, create)?;
    match statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        // The leaf is judged from its own lstat, never from the open
        // that would follow it: a link planted here is refused, not
        // resolved to a target the account does not own.
        Ok(meta)
            if rustix::fs::FileType::from_raw_mode(meta.st_mode)
                == rustix::fs::FileType::Symlink =>
        {
            Err(format!("user: home {home} exists and is a symlink"))
        }
        Ok(_) => open_leaf(&dir, name, home),
        Err(rustix::io::Errno::NOENT) if create => {
            mkdirat(&dir, name, HOME_MODE)
                .map_err(|error| format!("user: create home {home}: {error}"))?;
            let leaf = open_leaf(&dir, name, home)?;
            enforce_mode(&leaf, HOME_MODE, home)?;
            Ok(leaf)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

/// The mode the directory is given: the mode the creation asked for is
/// only a request — the process's umask takes bits away from it, and a
/// default ACL on the parent grants the ones the request does not ask
/// for — so the mode is enforced on the directory itself, after it is
/// created.
fn enforce_mode(fd: &OwnedFd, mode: Mode, home: &str) -> Result<(), String> {
    fchmod(fd, mode).map_err(|error| format!("user: create home {home}: {error}"))
}

fn open_leaf(dir: &OwnedFd, name: &OsStr, home: &str) -> Result<OwnedFd, String> {
    openat(dir, name, walk_flags(), Mode::empty())
        .map_err(|error| format!("user: own home {home}: {error}"))
}

/// The flags every directory of the walk opens with: never following a
/// link, so a component replaced after `ancestors_usable` inspected the
/// path cannot redirect the operation to a target the account does not
/// own.
fn walk_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Every directory of `path` opened from the root down, one component
/// at a time through `openat` — creating the missing ones when
/// `create`. The walk is anchored to the inodes it opened: a directory
/// renamed aside and replaced after the walk passed it leaves the
/// operation on the original inode, and a link planted where a
/// non-root principal could rewrite it is refused rather than
/// followed. A link only root can rewrite is the operator's
/// configuration (`/var` on macOS, `/home`) — the admission
/// `planted_in` grants it — and is resolved by its target, walked
/// under the same no-follow rule.
pub(super) fn open_chain(path: &Path, home: &str, create: bool) -> Result<OwnedFd, String> {
    let mut names: Vec<OsString> = Vec::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => names.push(name.to_os_string()),
            Component::ParentDir => names.push(OsString::from("..")),
            Component::Prefix(_) => return Err(format!("user: home {home}: invalid path")),
        }
    }
    let mut current = open(
        if path.is_absolute() { "/" } else { "." },
        walk_flags(),
        Mode::empty(),
    )
    .map_err(|error| format!("user: home {home}: {error}"))?;
    let mut expansions = 0;
    let mut index = 0;
    while index < names.len() {
        let name = names[index].clone();
        match openat(&current, &name, walk_flags(), Mode::empty()) {
            Ok(fd) => {
                current = fd;
                index += 1;
            }
            Err(rustix::io::Errno::NOENT) if create => {
                mkdirat(&current, &name, PARENT_MODE).map_err(|error| {
                    format!(
                        "user: create home {home}: {}: {error}",
                        Path::new(&name).display()
                    )
                })?;
                current = openat(&current, &name, walk_flags(), Mode::empty())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                enforce_mode(&current, PARENT_MODE, home)?;
                index += 1;
            }
            Err(rustix::io::Errno::NOENT) => {
                return Err(format!(
                    "user: home {home}: {} is missing",
                    Path::new(&name).display()
                ));
            }
            // A link the no-follow open refuses answers `ELOOP` on
            // Linux and `ENOTDIR` on the BSDs (a link under
            // `O_DIRECTORY`), so the component's own lstat decides
            // what the refusal was: a link is resolved under the
            // admission rule, anything else is not a directory.
            Err(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) => {
                let meta = statat(&current, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                if rustix::fs::FileType::from_raw_mode(meta.st_mode)
                    != rustix::fs::FileType::Symlink
                {
                    return Err(format!(
                        "user: home {home}: {} is not a directory",
                        Path::new(&name).display()
                    ));
                }
                // The link is admitted only where it stands: its
                // directory is root's and not rewritable, the rule
                // `planted_in` applies to the path-based walk.
                let parent =
                    fstat(&current).map_err(|error| format!("user: home {home}: {error}"))?;
                if parent.st_uid != 0 || parent.st_mode & 0o022 != 0 {
                    return Err(format!(
                        "user: home {home}: {} is a symlink in a directory a non-root principal can rewrite",
                        Path::new(&name).display()
                    ));
                }
                expansions += 1;
                if expansions > 8 {
                    return Err(format!("user: home {home}: too many symlink expansions"));
                }
                let target = readlinkat(&current, &name, Vec::new())
                    .map_err(|error| format!("user: home {home}: {error}"))?;
                let target = OsString::from_vec(target.into_bytes());
                if Path::new(&target).is_absolute() {
                    current = open("/", walk_flags(), Mode::empty())
                        .map_err(|error| format!("user: home {home}: {error}"))?;
                }
                let mut replacement: Vec<OsString> = Path::new(&target)
                    .components()
                    .filter_map(|part| match part {
                        Component::Normal(part) => Some(part.to_os_string()),
                        Component::ParentDir => Some(OsString::from("..")),
                        _ => None,
                    })
                    .collect();
                replacement.extend(names[index + 1..].iter().cloned());
                names.truncate(index);
                names.extend(replacement);
            }
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(current)
}

/// The ownership the walked directory is given, on the descriptor the
/// walk opened: the fd names the inode the path resolved to when the
/// walk opened it, so a rewrite after that point changes nothing about
/// which directory is owned.
fn own_through(fd: &OwnedFd, uid: u32, gid: u32, home: &str) -> Result<(), String> {
    let meta = fstat(fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if meta.st_uid != uid || meta.st_gid != gid {
        fchown(fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|error| format!("user: own home {home}: {error}"))?;
    }
    Ok(())
}

/// The adoption of an existing home: the directory a non-root
/// principal may have placed is refused, the operator's own staging is
/// adopted — over scratch directories, never a host account.
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    /// A scratch directory of the test's own: unique to the test, so
    /// tests running beside it cannot collide with it.
    fn scratch(scenario: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ruxel-adopt-{scenario}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The identity the test process owns: a probe file of it, so the
    /// tests stage the operator's own directories without privilege.
    fn own_identity() -> (u64, u64) {
        let probe = std::env::temp_dir().join(format!(
            "ruxel-adopt-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&probe, b"").unwrap();
        let meta = std::fs::metadata(&probe).unwrap();
        let identity = (u64::from(meta.uid()), u64::from(meta.gid()));
        std::fs::remove_file(&probe).unwrap();
        identity
    }

    #[test]
    fn an_attacker_writable_home_is_refused_never_adopted() {
        // The shared-writable parent (`/tmp`) with a pre-created home
        // in it: a non-root principal may have placed the directory
        // and filled it — the planted login script is the payload the
        // new account would then execute. The agent must refuse the
        // adoption in every mode instead of changing the ownership and
        // keeping the contents.
        let parent = scratch("attack");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        let home = parent.join("alice");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::fs::write(home.join(".profile"), b"evil\n").unwrap();
        let (uid, gid) = own_identity();
        // The preflight refuses before any mutation.
        let error = home_usable(home.to_str().unwrap())
            .expect_err("a writable pre-created home must be refused");
        assert!(error.contains("adopt"), "{error}");
        // The repair refuses the same way in both modes.
        for check_mode in [true, false] {
            let error = ensure_home(home.to_str().unwrap(), uid, gid, check_mode)
                .expect_err("a writable pre-created home must never be adopted");
            assert!(error.contains("adopt"), "{error}");
        }
        // The creation path refuses the same way and takes the account
        // back instead of leaving it pointed at the planted directory.
        let taken: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
        let error = reconcile_created_home(
            "ruxel-adopt-alice",
            home.to_str().unwrap(),
            &super::super::entries::PasswdEntry {
                uid,
                gid,
                ..Default::default()
            },
            false,
            &|account| {
                taken.borrow_mut().push(account.to_string());
                Ok(())
            },
        )
        .expect_err("the creation must not adopt the planted home");
        assert!(error.contains("adopt"), "{error}");
        assert_eq!(
            taken.borrow().clone(),
            vec!["ruxel-adopt-alice".to_string()],
            "the account the step created is taken back"
        );
        // Nothing was adopted: the ownership and the planted contents
        // are exactly what the principal left.
        let meta = std::fs::metadata(&home).unwrap();
        assert_eq!(
            (u64::from(meta.uid()), u64::from(meta.gid())),
            (uid, gid),
            "the planted directory keeps its owner"
        );
        assert_eq!(
            std::fs::read(home.join(".profile")).unwrap(),
            b"evil\n",
            "the planted contents are preserved, never handed over"
        );
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn an_operator_placed_home_is_adopted() {
        // The operator's own staging under a shared path: the
        // directory the operator placed — owned by the operating
        // principal and writable by no one else — is adopted, with its
        // contents. `useradd -m` leaves a pre-existing directory's
        // ownership untouched, so the reconciliation repairs it with
        // the account's final identity instead of refusing it.
        let parent = scratch("operator");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        let home = parent.join("alice");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
        let (uid, gid) = own_identity();
        // The preflight admits the operator's staging.
        home_usable(home.to_str().unwrap()).expect("the operator's home must be usable");
        // Check mode reports the repair where the ownership drifted.
        assert!(
            ensure_home(home.to_str().unwrap(), uid + 1, gid + 1, true).unwrap(),
            "the drifted operator home reports its repair"
        );
        // Apply mode with matching ownership is no change — accepted,
        // nothing to do — and keeps the skeleton.
        assert!(
            !ensure_home(home.to_str().unwrap(), uid, gid, false).unwrap(),
            "the operator's home is adopted, not refused"
        );
        assert_eq!(
            std::fs::read(home.join(".profile")).unwrap(),
            b"operator skeleton\n",
            "the operator's contents survive the adoption"
        );
        // The creation path adopts it the same way.
        let error: Result<bool, String> = reconcile_created_home(
            "ruxel-adopt-alice",
            home.to_str().unwrap(),
            &super::super::entries::PasswdEntry {
                uid,
                gid,
                ..Default::default()
            },
            false,
            &|_| panic!("nothing to take back: the adoption succeeds"),
        );
        assert!(error.is_ok(), "the creation adopts the operator's home");
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn an_attacker_owned_home_under_a_shared_path_is_refused() {
        // The finding's exact shape: the declared home pre-created by
        // another uid under a shared-writable path. Only the root lane
        // can stage another uid's directory, so the unprivileged lanes
        // skip it; the root lane proves the refusal the pre-created
        // directory earns whatever its mode is.
        if super::operating_uid() != 0 {
            return;
        }
        let parent = scratch("attack-owned");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        let home = parent.join("alice");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join(".profile"), b"evil\n").unwrap();
        // Another principal's directory: the nobody account's numbers.
        let nobody = 65534u32;
        rustix::fs::chown(
            &home,
            Some(rustix::fs::Uid::from_raw(nobody)),
            Some(rustix::fs::Gid::from_raw(nobody)),
        )
        .unwrap();
        let error = home_usable(home.to_str().unwrap())
            .expect_err("another principal's pre-created home must be refused");
        assert!(error.contains("adopt"), "{error}");
        for check_mode in [true, false] {
            let error = ensure_home(home.to_str().unwrap(), 0, 0, check_mode)
                .expect_err("another principal's pre-created home must never be adopted");
            assert!(error.contains("adopt"), "{error}");
        }
        let meta = std::fs::metadata(&home).unwrap();
        assert_eq!(
            meta.uid(),
            nobody,
            "the refused directory is never handed to the account"
        );
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn an_operator_owned_home_under_a_shared_path_is_repaired() {
        // The root lane's adoption: the operator's own directory under
        // a shared path is repaired with the account's identity — the
        // ownership moves and the operator's contents stay.
        if super::operating_uid() != 0 {
            return;
        }
        let parent = scratch("operator-owned");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        let home = parent.join("alice");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
        let repaired = ensure_home(home.to_str().unwrap(), 65534, 65534, false)
            .expect("the operator's home must be adopted");
        assert!(repaired, "the ownership moves to the account");
        let meta = std::fs::metadata(&home).unwrap();
        assert_eq!(
            (meta.uid(), meta.gid()),
            (65534, 65534),
            "the directory belongs to the account now"
        );
        assert_eq!(
            std::fs::read(home.join(".profile")).unwrap(),
            b"operator skeleton\n",
            "the operator's contents survive the adoption"
        );
        std::fs::remove_dir_all(&parent).unwrap();
    }
}

#[cfg(test)]
mod ancestor_tests {
    /// The ancestor walk itself: a shared-writable ancestor refuses the
    /// adoption whatever stands beneath it, and the operator's own tree
    /// accepts it — over paths the walk only inspects, never changes.
    #[test]
    fn untrusted_ancestors_refuse_the_adoption() {
        // The shared directory itself, resolved past any symlink: it
        // is world-writable on every platform (1777 on Linux, 1777
        // through the `/tmp` symlink on macOS), so a home beneath it
        // may have been placed by any principal. The intermediate
        // directory exists, so the walk reaches the untrusted ancestor
        // instead of ending at a missing component.
        let shared = std::fs::canonicalize("/tmp")
            .expect("the shared directory resolves")
            .join(format!("ruxel-adopt-ancestors-{}", std::process::id()));
        std::fs::create_dir_all(&shared).unwrap();
        let error = super::trusted_ancestors(shared.join("alice").to_str().unwrap())
            .expect_err("a home under a shared-writable ancestor must be refused");
        assert!(error.contains("refusing to adopt"), "{error}");
        std::fs::remove_dir_all(&shared).unwrap();
        // The operator's own tree accepts it: a directory the process
        // owns, writable by no one else, under ancestors that are the
        // operator's or root's and writable by no one else.
        let anchor = std::env::home_dir()
            .expect("the test process runs with a home directory")
            .join(format!("ruxel-adopt-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&anchor);
        std::fs::create_dir(&anchor).unwrap();
        super::trusted_ancestors(anchor.join("alice").to_str().unwrap())
            .expect("the operator's own tree must accept the adoption");
        std::fs::remove_dir_all(&anchor).unwrap();
    }
}

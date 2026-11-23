//! The home `useradd -m` created for a new account, taken back from
//! the skeleton template it was populated with.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, open, openat, readlinkat, statat, unlinkat};

/// The skeleton directory `useradd -m` populates a created home from
/// where the host configures none: the shadow default, copied into
/// every new home where it is non-empty.
const DEFAULT_SKEL_DIR: &str = "/etc/skel";

/// The skeleton directory `useradd -m` populates a created home from:
/// the host's configured `SKEL` in `/etc/default/useradd` (`KEY=VALUE`,
/// the `useradd -D` defaults file), the shadow default where the host
/// configures none. An unreadable policy fails to the default:
/// `useradd` would populate from it in the same case.
pub(crate) fn skel_dir() -> String {
    match std::fs::read_to_string("/etc/default/useradd") {
        Ok(content) => skel_dir_from(&content),
        Err(_) => DEFAULT_SKEL_DIR.to_string(),
    }
}

/// The canned-content counterpart of [`skel_dir`]: the last uncommented
/// `SKEL=...` line wins, and anything else — including absent — is the
/// shadow default. Values may be quoted; an empty assignment keeps the
/// previous value.
pub(crate) fn skel_dir_from(content: &str) -> String {
    let mut skel = DEFAULT_SKEL_DIR.to_string();
    for line in content.lines() {
        let stripped = line.split('#').next().unwrap_or("").trim();
        if stripped.is_empty() {
            continue;
        }
        let assignment = stripped.strip_prefix("export ").map_or(stripped, str::trim);
        if let Some(value) = assignment.strip_prefix("SKEL=") {
            let value = value
                .trim()
                .trim_matches(|cell: char| cell == '"' || cell == '\'')
                .trim();
            if !value.is_empty() {
                skel = value.to_string();
            }
        }
    }
    skel
}

/// Whether the skeleton directory differs from the shadow default: only
/// a configured template is passed to `useradd -k` explicitly — the
/// default rides along with `-m` the way it does today.
pub(crate) fn skel_configured(skel: &str) -> bool {
    skel != DEFAULT_SKEL_DIR
}

/// The home `useradd -m` created for a new account, taken back: the
/// directory the operation added goes with the account, while contents
/// a concurrent writer planted are kept and reported rather than
/// deleted. Only a home this operation created is removed.
pub(crate) fn rollback_created_home(home: &str) -> Result<(), String> {
    rollback_created_home_from(home, Path::new(&skel_dir()))
}

/// The canned-skel counterpart of [`rollback_created_home`]: the
/// skeleton root is the test's own, so populated-skel and
/// planted-content cases run without touching the host's `/etc/skel`.
pub(crate) fn rollback_created_home_from(home: &str, skel: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(home)
        .map_err(|error| format!("user: take back created home {home}: {error}"))?;
    if meta.is_symlink() {
        return Err(format!(
            "user: take back created home {home}: home is a symlink"
        ));
    }
    if !meta.is_dir() {
        return Err(format!(
            "user: take back created home {home}: home is not a directory"
        ));
    }
    let home_set = relative_entries(Path::new(home))
        .map_err(|error| format!("user: take back created home {home}: {error}"))?;
    // Provenance is the template's own entry at the name — the same kind
    // of entry with the same bytes — not the name alone: an overwrite or
    // a planted link at a template name is foreign, so the home is kept.
    for entry in &home_set {
        if !skel_entry_matches(Path::new(home), skel, entry)? {
            return Err(format!(
                "user: take back created home {home}: home contains contents a concurrent writer planted"
            ));
        }
    }
    remove_home_entries(home, skel, &home_set)
}

/// Takes back a scanned home entry by entry, deepest first: each entry
/// is proven the template's own again just before it is removed, so a
/// concurrent writer that planted after the scan stops the rollback
/// instead of being deleted — and the final removal is non-recursive,
/// so a concurrent addition makes the rollback stop rather than
/// deleting it. Entries are removed by identity, never through a
/// link: a symlink is unlinked itself, and a directory removal fails
/// on anything planted inside it after its proof.
pub(crate) fn remove_home_entries(
    home: &str,
    skel: &Path,
    entries: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    // Both roots are pinned before the first entry is touched: every
    // proof and every removal below runs against these descriptors, so
    // a concurrent writer that swaps an intermediate directory for a
    // symlink after the scan cannot redirect the proof at the
    // skeleton itself — the no-follow open of the swapped component
    // fails and the rollback stops with the target untouched.
    let home_fd = open_pinned_dir(Path::new(home), home)?;
    if entries.is_empty() {
        // Nothing to prove: the scan found no entry, so no skeleton
        // read decides anything — the empty directory the operation
        // created is taken back whatever template it came from.
        drop(home_fd);
        std::fs::remove_dir(home)
            .map_err(|error| format!("user: take back created home {home}: {error}"))?;
        return Ok(());
    }
    let skel_fd = open_pinned_dir(skel, home)?;
    let mut ordered: Vec<&PathBuf> = entries.iter().collect();
    ordered.sort_by_key(|entry| std::cmp::Reverse(entry.components().count()));
    for entry in ordered {
        remove_pinned_entry(home, &home_fd, &skel_fd, entry)?;
    }
    drop(home_fd);
    drop(skel_fd);
    // The final removal stays a single non-recursive `remove_dir`: it
    // fails on a symlink and on anything planted inside after the
    // proof, so a concurrent addition keeps the home instead of being
    // deleted with it.
    std::fs::remove_dir(home)
        .map_err(|error| format!("user: take back created home {home}: {error}"))?;
    Ok(())
}

/// Opens a directory as itself: a trailing link is refused, never
/// resolved to a target the account does not own.
fn open_pinned_dir(path: &Path, home: &str) -> Result<OwnedFd, String> {
    open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        format!(
            "user: take back created home {home}: {}: {error}",
            path.display()
        )
    })
}

/// Proves one scanned entry against its parent descriptors and removes
/// it by identity, never through a link: the parent chain is reopened
/// from the pinned home for every entry, so a directory the scan saw
/// that a concurrent writer has since swapped for a symlink fails the
/// no-follow open and stops the rollback instead of proving — and
/// deleting — the link's target. A symlink is unlinked itself, and a
/// directory removal refuses anything planted inside it after its
/// proof.
fn remove_pinned_entry(
    home: &str,
    home_fd: &OwnedFd,
    skel_fd: &OwnedFd,
    entry: &Path,
) -> Result<(), String> {
    let planted = || {
        format!(
            "user: take back created home {home}: home contains contents a concurrent writer planted"
        )
    };
    let gone = |error: rustix::io::Errno| {
        format!(
            "user: take back created home {home}: {}: {error}",
            entry.display()
        )
    };
    let name: &OsStr = entry.file_name().ok_or_else(planted)?;
    let Some(parent) = entry.parent() else {
        return Err(planted());
    };
    let home_parent = open_pinned_chain(home_fd, parent).map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            gone(error)
        } else {
            planted()
        }
    })?;
    let skel_parent = open_pinned_chain(skel_fd, parent).map_err(|_| planted())?;
    // The leaf is judged from its own lstat against the parent
    // descriptor, never through a resolved pathname.
    let home_meta = statat(&home_parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            gone(error)
        } else {
            planted()
        }
    })?;
    let skel_meta = statat(&skel_parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| planted())?;
    let (home_kind, skel_kind) = (
        FileType::from_raw_mode(home_meta.st_mode),
        FileType::from_raw_mode(skel_meta.st_mode),
    );
    match (home_kind, skel_kind) {
        (FileType::Symlink, FileType::Symlink) => {
            let home_target = readlinkat(&home_parent, name, Vec::new()).map_err(|_| planted())?;
            let skel_target = readlinkat(&skel_parent, name, Vec::new()).map_err(|_| planted())?;
            if home_target != skel_target {
                return Err(planted());
            }
            unlinkat(&home_parent, name, AtFlags::empty())
        }
        (FileType::Directory, FileType::Directory) => {
            unlinkat(&home_parent, name, AtFlags::REMOVEDIR)
        }
        (FileType::RegularFile, FileType::RegularFile) => {
            if pinned_file_bytes(&home_parent, name).map_err(|_| planted())?
                != pinned_file_bytes(&skel_parent, name).map_err(|_| planted())?
            {
                return Err(planted());
            }
            unlinkat(&home_parent, name, AtFlags::empty())
        }
        _ => return Err(planted()),
    }
    .map_err(|error| {
        format!(
            "user: take back created home {home}: {}: {error}",
            entry.display()
        )
    })
}

/// Every directory of `parent` opened from the pinned root down
/// through `openat`, never following a link: a component a concurrent
/// writer replaced after the scan answers `ELOOP` (a link) or
/// `ENOTDIR` (a file where the scan saw a directory) and the caller
/// refuses instead of entering the replacement.
fn open_pinned_chain(root: &OwnedFd, parent: &Path) -> Result<OwnedFd, rustix::io::Errno> {
    use std::path::Component;
    let mut current: Option<OwnedFd> = None;
    for component in parent.components() {
        // Anything but a plain name fails closed: the scan only ever
        // yields names, so an escape here is tampering, not content.
        let Component::Normal(name) = component else {
            return Err(rustix::io::Errno::INVAL);
        };
        let base: BorrowedFd<'_> = current
            .as_ref()
            .map_or_else(|| root.as_fd(), OwnedFd::as_fd);
        current = Some(openat(
            base,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
    }
    // The chain borrows the root where the entry stands at its top:
    // reopening the root as itself keeps the leaf proof against the
    // pinned descriptor rather than a resolved pathname.
    match current {
        Some(fd) => Ok(fd),
        None => openat(
            root,
            c".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ),
    }
}

/// The bytes of the leaf the descriptor names, read from the open
/// file itself: the content the proof compares is the inode the
/// removal then unlinks by name.
fn pinned_file_bytes(dir: &OwnedFd, name: &OsStr) -> Result<Vec<u8>, String> {
    let fd = openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::fs::File::from(fd), &mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Whether the home's entry is the template's own at the same name: the same
/// kind of entry with the same bytes — an overwrite or a planted link is foreign.
fn skel_entry_matches(home: &Path, skel: &Path, entry: &Path) -> Result<bool, String> {
    let (at_home, at_skel) = (home.join(entry), skel.join(entry));
    let home_meta = std::fs::symlink_metadata(&at_home)
        .map_err(|error| format!("{}: {error}", at_home.display()))?;
    let Ok(skel_meta) = std::fs::symlink_metadata(&at_skel) else {
        return Ok(false);
    };
    if home_meta.is_symlink() != skel_meta.is_symlink() {
        return Ok(false);
    }
    if home_meta.is_symlink() {
        let home_target = std::fs::read_link(&at_home)
            .map_err(|error| format!("{}: {error}", at_home.display()))?;
        return Ok(std::fs::read_link(&at_skel).is_ok_and(|target| target == home_target));
    }
    if home_meta.is_dir() && skel_meta.is_dir() {
        return Ok(true);
    }
    if !home_meta.is_file() || !skel_meta.is_file() {
        return Ok(false);
    }
    let home_bytes =
        std::fs::read(&at_home).map_err(|error| format!("{}: {error}", at_home.display()))?;
    Ok(std::fs::read(&at_skel).is_ok_and(|skel_bytes| skel_bytes == home_bytes))
}

/// Every entry beneath `root`, relative to it, without following a
/// link: directories recurse as themselves, and a link at any level is
/// an entry of its own — never resolved to a target the account does
/// not own.
pub(crate) fn relative_entries(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut entries = BTreeSet::new();
    collect_relative(root, root, &mut entries)?;
    Ok(entries)
}

fn collect_relative(
    root: &Path,
    dir: &Path,
    entries: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let listing = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in listing {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let name = entry.file_name();
        let relative = dir
            .strip_prefix(root)
            .map(|prefix| prefix.join(&name))
            .map_err(|error| format!("{}: {error}", dir.display()))?;
        entries.insert(relative.clone());
        let full = dir.join(&name);
        let meta = std::fs::symlink_metadata(&full)
            .map_err(|error| format!("{}: {error}", full.display()))?;
        if meta.is_dir() {
            collect_relative(root, &full, entries)?;
        }
    }
    Ok(())
}

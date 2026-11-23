//! The declared home directory of an account is the agent's to create —
//! and to keep belonging to the account.

use rustix::fs::{Mode, fchmod, fstat};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The mode a created home directory gets: the account's own — the
/// schema exposes no home-mode field, so the mode is the agent's
/// policy (`useradd -D`'s `HOME_MODE`).
const HOME_MODE: Mode = Mode::from_raw_mode(0o750);

/// The mode a created parent of the home gets: traversable, and never
/// world-writable — a parent others may write is one they may replace.
const PARENT_MODE: Mode = Mode::from_raw_mode(0o755);

#[cfg(test)]
#[cfg(test)]
pub(super) use super::adoption::managed_claim;
#[cfg(test)]
pub(super) use super::adoption::operating_uid;
pub(super) use super::adoption::{
    ManagedAccount, adoption_pinned, adoption_trusted, ancestors_usable, home_usable,
    trusted_ancestors,
};

mod open;

#[cfg(test)]
pub(crate) use open::open_chain;
pub(crate) use open::{open_home, own_through};

/// The home `useradd` creates for an account: the declared one, or
/// the footprint's `/home/<name>` — passed explicitly, never left to
/// the host's configured base.
pub(super) fn claimed_home(home: Option<&str>, name: &str) -> String {
    home.map_or_else(|| format!("/home/{name}"), str::to_string)
}

/// The gid the home directory is created with: the requested primary
/// group's — `usermod -g` has already moved the account by the time
/// the directory is created — or the account's current one when the
/// step names no group. `None` is check mode's answer for a group that
/// is not on the host yet (see `planned_gid`).
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
/// agent's to create — and one that drifted to another gid is the
/// agent's to reconcile. A symlink at the path is refused, every
/// ancestor is inspected before the path is traversed, a missing home
/// is created only under trusted ancestors, and ownership changes
/// never follow a link, so no link can hand the account a target. The
/// mutation runs on descriptors the walk opened without following a
/// link. Every directory the repair creates is collected for the
/// caller: a later failure takes back exactly what the repair added.
/// The adoption admits the directory only for the managed account.
pub(super) fn ensure_home(
    home: &str,
    uid: u64,
    gid: u64,
    check_mode: bool,
    managed: &ManagedAccount<'_>,
    created: &mut Vec<PathBuf>,
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
            adoption_trusted(home, &meta, managed)?;
            if meta.uid() == uid && meta.gid() == gid {
                return Ok(false);
            }
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, false, created)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the path the check above inspected may have been
            // replaced in between.
            adoption_pinned(home, &fd, managed)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A missing home is created beneath its ancestors — and
            // then owned — so the creation is admitted only under
            // trusted ones, the way an adoption is, in check mode as
            // much as in apply.
            trusted_ancestors(home)?;
            if check_mode {
                return Ok(true);
            }
            let fd = open_home(home, true, created)?;
            // The adoption the descriptors pin, re-verified after the
            // open: the leaf the lookup found missing may have been
            // created in between — a directory a non-root principal
            // planted must never become the account's home with its
            // contents. A directory the creation just made is the
            // operator's own and passes.
            adoption_pinned(home, &fd, managed)?;
            own_through(&fd, uid, gid, home)?;
            Ok(true)
        }
        Err(error) => Err(format!("user: own home {home}: {error}")),
    }
}

/// The claimed home of the account the step created: reconciled, and
/// the account taken back when the reconciliation fails.
pub(super) fn reconcile_created_home(
    name: &str,
    home: &str,
    created: &super::PasswdEntry,
    check_mode: bool,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    // The created account claims the home; an oversized number matches none.
    let managed = ManagedAccount {
        name,
        uid: u32::try_from(created.uid).ok(),
    };
    let mut added = Vec::new();
    // The ownership the home carries before the repair changes
    // anything: a trusted pre-existing home goes back to it when the
    // reconciliation fails, instead of staying owned by the deleted
    // account's number the way an unrecorded home does.
    let prior = prior_ownership(home);
    let reconciled: Result<bool, String> = (|| {
        let ownership = ensure_home(
            home,
            created.uid,
            created.gid,
            check_mode,
            &managed,
            &mut added,
        )?;
        let mode = enforce_created_mode(home, check_mode)?;
        Ok(ownership || mode)
    })();
    match reconciled {
        Ok(changed) => Ok(changed),
        Err(error) => {
            // Parents the failed repair created before it failed go
            // with the account: a retry must not inherit the partial
            // hierarchy the refused change leaves behind — and a
            // pre-existing home goes back to the ownership the repair
            // found, but only where the repair moved it.
            let error = restore_after_failure(home, prior, &added, error);
            match remove_account(name) {
                Ok(()) => Err(error),
                Err(rollback) => Err(format!(
                    "{error}; the account {name} could not be taken back: {rollback}"
                )),
            }
        }
    }
}

/// The ownership the claimed home carries before the reconciliation
/// changes anything: what a failed reconciliation restores — `None`
/// where the home is not there yet, so the rollback takes back only
/// what the repair created.
fn prior_ownership(home: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_dir() => Some((u64::from(meta.uid()), u64::from(meta.gid()))),
        _ => None,
    }
}

/// The home's state a failed reconciliation leaves behind, taken back:
/// a pre-existing home goes back to the ownership the repair found —
/// but only where the repair or anything racing it actually moved it.
/// A refusal before any change keeps its own error: a restoration the
/// home does not need would only append a take-back failure to a
/// refusal that changed nothing.
fn restore_after_failure(
    home: &str,
    prior: Option<(u64, u64)>,
    added: &[PathBuf],
    error: String,
) -> String {
    if added.is_empty() && prior == prior_ownership(home) {
        return error;
    }
    match super::apply::rollback_home(home, prior, added) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; {cleanup}"),
    }
}

/// The mode of the claimed home of an account the step created: the
/// policy's, not the host's creation default — read off the directory
/// itself after the reconciliation owns it.
fn enforce_created_mode(home: &str, check_mode: bool) -> Result<bool, String> {
    let fd = open_home(home, false, &mut Vec::new())?;
    let meta = fstat(&fd).map_err(|error| format!("user: own home {home}: {error}"))?;
    if Mode::from_raw_mode(meta.st_mode & 0o777) == HOME_MODE {
        return Ok(false);
    }
    if check_mode {
        return Ok(true);
    }
    fchmod(&fd, HOME_MODE).map_err(|error| format!("user: create home {home}: {error}"))?;
    Ok(true)
}

/// The adoption of an existing home: the directory a non-root
/// principal may have placed is refused, the operator's own staging is
/// adopted — over scratch directories, never a host account.
#[cfg(test)]
#[path = "home/tests/adoption.rs"]
mod tests;

#[cfg(test)]
#[path = "home/tests/ancestors.rs"]
mod ancestor_tests;

/// The pre-existing home a failed reconciliation leaves behind: the
/// ownership the repair found is restored, over scratch directories.
#[cfg(test)]
#[path = "home/tests/restore.rs"]
mod restore_tests;

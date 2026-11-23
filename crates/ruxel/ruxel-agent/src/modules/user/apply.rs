//! The changes an existing account's step runs, in the order they run:
//! the private group the step claims, the home's repair, and the
//! account's own change.

use std::path::PathBuf;

use super::PasswdEntry;
use super::entries::group_entry;
use super::guards::identity_change_allowed;
use super::home::{ManagedAccount, ensure_home};

mod rollback_created;

#[cfg(test)]
pub(crate) use rollback_created::{
    relative_entries, remove_home_entries, rollback_created_home_from, skel_dir_from,
};
pub(crate) use rollback_created::{rollback_created_home, skel_configured, skel_dir};

/// The command the agent runs for the account's change.
pub(super) type Runner = dyn Fn(&str, &[&str]) -> Result<(), String>;

/// The number the group that owns the home holds under the plan.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HomeOwner {
    /// The number the plan knows for the group the home belongs to.
    Number(u64),
    /// The account's own same-name group, whose number the plan's own
    /// group creation draws at the apply: the number is unknown until
    /// the group exists.
    SameName,
}

/// The home's repair the step plans: the directory the account's home
/// field points at, created and owned by the account.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Home {
    pub(super) path: String,
    pub(super) uid: u64,
    pub(super) owner: HomeOwner,
    /// The account's own number the adoption weighs: the plan's where
    /// it states one — an allocator-drawn number matches no claim.
    pub(super) managed_uid: u64,
}

/// The changes the existing account's branch runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Mutation {
    /// The private group the step claims, created before the account's
    /// own change: `usermod -G` resolves the supplementary list against
    /// the host, and the list may name the group.
    GroupAdd(String),
    /// The home's repair. It runs before the account's own change: it
    /// is that change's preflight — a repair that fails must leave
    /// the account as it was.
    Home(Home),
    /// The account's own change.
    UserMod(Vec<String>),
}

/// The changes the existing account's step runs. The identity guard
/// runs here, before the first of them: a refused change plans no
/// mutation at all.
pub(super) fn mutations(
    name: &str,
    current: &PasswdEntry,
    identity_change: bool,
    group_missing: bool,
    home: Option<Home>,
    usermod: &[&str],
) -> Result<Vec<Mutation>, String> {
    if identity_change {
        // The live uid-0 account is the superuser's whatever it is
        // named: moving it off uid 0 — or off its primary group —
        // strands every file it owns.
        identity_change_allowed(name, current)?;
    }
    let mut mutations = Vec::new();
    if group_missing {
        mutations.push(Mutation::GroupAdd(name.to_string()));
    }
    if let Some(home) = home {
        mutations.push(Mutation::Home(home));
    }
    if !usermod.is_empty() {
        mutations.push(Mutation::UserMod(
            usermod
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
        ));
    }
    Ok(mutations)
}

/// A mutation the apply has already performed: what a later failure
/// takes back, so a refused change leaves the host as it was. The
/// mutation itself can still fail after changing the host — an
/// immutable directory, a read-only parent.
enum Completed {
    /// The private group the step created: taken back with `groupdel`.
    Group(String),
    /// The home's repair: the ownership the repair found — restored —
    /// or the directories the repair created — removed leaf-first.
    Home {
        path: String,
        prior: Option<(u64, u64)>,
        created: Vec<PathBuf>,
    },
}

/// The changes the existing account's step runs, in the order the plan
/// made them. The home's repair is the preflight of the account's own
/// change, so a repair that fails leaves the account as it was. A
/// mutation that fails takes back what the earlier ones did, so a
/// refused change leaves the host as it was. Returns whether the
/// home's repair changed (or would change in check mode) anything.
pub(super) fn run(
    name: &str,
    mutations: Vec<Mutation>,
    check_mode: bool,
    run_cmd: &Runner,
) -> Result<bool, String> {
    let mut changed = false;
    let mut completed: Vec<Completed> = Vec::new();
    for mutation in mutations {
        let step: Result<bool, String> = (|| {
            match mutation {
                Mutation::GroupAdd(group) => {
                    // The allocator draws the recreated number: a file
                    // the group owned keeps the old one, so the plan's
                    // record of the moved number is what tells an
                    // ownership cache it is stale — the new-account
                    // branch marks its implicit creation the same way.
                    // The mark lands before the creation runs, the way
                    // check mode records before it executes: a failed
                    // `groupadd` only over-invalidates, never wrongly
                    // validates a cache.
                    super::plan::note_changed_group(&group);
                    if !check_mode {
                        run_cmd("groupadd", &[&group])?;
                        completed.push(Completed::Group(group));
                    }
                    Ok(false)
                }
                Mutation::Home(home) => {
                    let Some(gid) = owner_gid(&home.owner, name)? else {
                        return Ok(false);
                    };
                    // The ownership the repair finds: a later failure
                    // takes the repair back to it — or takes the created
                    // directories away where the repair created them.
                    // Check mode mutates nothing, so there is nothing to
                    // take back.
                    let prior = if check_mode {
                        None
                    } else {
                        prior_owner(&home.path)?
                    };
                    // The account the repair adopts for, and every
                    // directory the repair creates: the rollback below
                    // takes back the partial hierarchy of a repair that
                    // fails after creating parents, so it must not
                    // survive the refused change.
                    let managed = ManagedAccount {
                        name,
                        uid: u32::try_from(home.managed_uid).ok(),
                    };
                    let mut created = Vec::new();
                    let outcome = ensure_home(
                        &home.path,
                        home.uid,
                        gid,
                        check_mode,
                        &managed,
                        &mut created,
                    );
                    if !check_mode {
                        completed.push(Completed::Home {
                            path: home.path,
                            prior,
                            created,
                        });
                    }
                    outcome
                }
                Mutation::UserMod(mut args) => {
                    if !check_mode {
                        args.push(name.to_string());
                        let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
                        run_cmd("usermod", &arguments)?;
                    }
                    Ok(false)
                }
            }
        })();
        match step {
            Ok(repaired) => changed |= repaired,
            Err(error) => return Err(take_back(&completed, run_cmd, error)),
        }
    }
    Ok(changed)
}

/// The home's repair a later failure takes back: the ownership the
/// repair found, restored — or every directory the repair created,
/// removed leaf-first. The removal refuses a directory that is not
/// empty, so contents a concurrent writer planted are kept and
/// reported rather than deleted.
pub(super) fn rollback_home(
    path: &str,
    prior: Option<(u64, u64)>,
    created: &[PathBuf],
) -> Result<(), String> {
    let Some((uid, gid)) = prior else {
        // Every parent directory the repair created, leaf first: the
        // removal refuses a directory that is not empty, so contents
        // a concurrent writer planted are kept and reported rather
        // than deleted.
        let mut targets = created.to_vec();
        if targets.is_empty() {
            targets.push(PathBuf::from(path));
        }
        let mut first: Option<String> = None;
        for dir in targets.iter().rev() {
            if let Err(error) = std::fs::remove_dir(dir)
                && first.is_none()
            {
                first = Some(format!(
                    "user: take back created home {}: {error}",
                    dir.display()
                ));
            }
        }
        match first {
            Some(error) => return Err(error),
            None => return Ok(()),
        }
    };
    let uid = u32::try_from(uid)
        .map_err(|_| format!("user: take back home {path}: uid {uid} does not fit"))?;
    let gid = u32::try_from(gid)
        .map_err(|_| format!("user: take back home {path}: gid {gid} does not fit"))?;
    let fd = super::home::open_home(path, false, &mut Vec::new())?;
    super::home::own_through(&fd, uid, gid, path)
}

/// What the earlier mutations did, taken back in reverse order. A
/// rollback that fails itself is reported with the error that caused
/// it: the host keeps the partial change, and the task says so
/// instead of reporting a clean refusal.
fn take_back(completed: &[Completed], run_cmd: &Runner, error: String) -> String {
    let mut failures = Vec::new();
    for done in completed.iter().rev() {
        let undone = match done {
            Completed::Group(group) => run_cmd("groupdel", &[group]).map_err(|rollback| {
                format!("the group {group} could not be taken back: {rollback}")
            }),
            Completed::Home {
                path,
                prior,
                created,
            } => { rollback_home(path, *prior, created) }
                .map_err(|rollback| format!("the home {path} could not be taken back: {rollback}")),
        };
        if let Err(failure) = undone {
            failures.push(failure);
        }
    }
    if failures.is_empty() {
        error
    } else {
        format!("{error}; {}", failures.join("; "))
    }
}

/// The number the home's owner group holds: the number the plan knows,
/// or the number its own group creation draws — `None` where the group
/// is not on the host and the plan creates none.
fn owner_gid(owner: &HomeOwner, name: &str) -> Result<Option<u64>, String> {
    match owner {
        HomeOwner::Number(gid) => Ok(Some(*gid)),
        HomeOwner::SameName => Ok(group_entry(name)?.map(|(_, gid)| gid)),
    }
}

/// The ownership the home's repair finds, before it changes anything:
/// what a later failure restores — `None` where the home is not there
/// yet, so the repair creates it and the rollback takes it away.
fn prior_owner(home: &str) -> Result<Option<(u64, u64)>, String> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_dir() => Ok(Some((u64::from(meta.uid()), u64::from(meta.gid())))),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("user: repair home {home}: {error}")),
    }
}

/// The skeleton-provenance rollback: overwrites and planted links are foreign.
#[cfg(test)]
#[path = "apply/tests/rollback_skel.rs"]
mod skel_tests;
/// The rollback of a failed change: the mutations a later step's
/// failure takes back, over canned commands and scratch directories —
/// no host account is touched.
#[cfg(test)]
#[path = "apply/tests/rollback.rs"]
mod tests;

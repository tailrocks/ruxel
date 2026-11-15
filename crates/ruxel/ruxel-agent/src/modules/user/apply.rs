//! The changes an existing account's step runs, in the order they run:
//! the private group the step claims, the home's repair, and the
//! account's own change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::PasswdEntry;
use super::entries::group_entry;
use super::guards::identity_change_allowed;
use super::home::ensure_home;

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
}

/// The changes the existing account's branch runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Mutation {
    /// The private group the step claims, created before the account's
    /// own change: `usermod -G` resolves the supplementary list against
    /// the host, and the list may name the group.
    GroupAdd(String),
    /// The home's repair. It runs before the account's own change: it
    /// is that change's preflight — a repair that fails (an immutable
    /// directory, a read-only parent, an identity that does not fit)
    /// must leave the account as it was, not with its passwd field
    /// moved onto a home that does not exist and its identity
    /// changed.
    Home(Home),
    /// The account's own change.
    UserMod(Vec<String>),
}

/// The changes the existing account's step runs. The identity guard
/// runs here, before the first of them: a refused change plans no
/// mutation at all, so the host keeps the private group the step
/// would have created, exactly as it kept it before the refusal.
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
/// path-shape preflight (`home_usable`) runs before any mutation, but
/// the mutation itself can still fail — an immutable directory, a
/// read-only parent — and the mutations before it have already changed
/// the host by then.
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
/// made them. The home's repair runs before the account's own change:
/// it is the preflight of that change, so a repair that fails leaves
/// the account as it was. A mutation that fails takes back what the
/// earlier ones did — the created group, the repaired home with every
/// parent directory the repair created — so a refused change leaves
/// the host as it was. Returns whether the
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
                    let (changed, created) = ensure_home(&home.path, home.uid, gid, check_mode)?;
                    if !check_mode {
                        completed.push(Completed::Home {
                            path: home.path,
                            prior,
                            created,
                        });
                    }
                    Ok(changed)
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

/// The skeleton directory `useradd -m` populates a created home from:
/// the host's own template, copied into every new home where it is
/// non-empty.
const SKEL_DIR: &str = "/etc/skel";

/// The home `useradd -m` created for a new account, taken back: the
/// directory the operation added goes with the account — including the
/// skeleton contents `useradd` copied from it — while contents a
/// concurrent writer planted are kept and reported rather than deleted.
/// Only a home this operation created is removed: the caller checks it
/// did not pre-exist, and a home that pre-existed is the host's,
/// whatever it holds.
pub(super) fn rollback_created_home(home: &str) -> Result<(), String> {
    rollback_created_home_from(home, Path::new(SKEL_DIR))
}

/// The canned-skel counterpart of [`rollback_created_home`]: the
/// skeleton root is the test's own, so populated-skel and
/// planted-content cases run without touching the host's `/etc/skel`.
pub(super) fn rollback_created_home_from(home: &str, skel: &Path) -> Result<(), String> {
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
    std::fs::remove_dir_all(home)
        .map_err(|error| format!("user: take back created home {home}: {error}"))?;
    Ok(())
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
fn relative_entries(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
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
/// The home's repair a later failure takes back: the ownership the
/// repair found, restored — or every directory the repair created,
/// removed leaf-first. The reversal runs on the descriptors the safe walk opens,
/// never through a link a concurrent rewrite plants, for the repair's
/// own reason. Removing the created directory takes back exactly what
/// the repair added: the removal refuses a directory that is not
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
    let (fd, _) = super::home::open_home(path, false)?;
    super::home::own_through(&fd, uid, gid, path)
}

/// What the earlier mutations did, taken back in reverse order: the
/// private group the step created is deleted, and the home's repair is
/// reversed — the ownership it found restored, every directory it
/// created removed leaf-first. A rollback that fails itself is reported with the error
/// that caused it: the host keeps the partial change, and the task
/// says so instead of reporting a clean refusal.
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
/// and the number the plan's own group creation draws where the group
/// does not exist yet — `None` where the group is not on the host and
/// the plan creates none, which is check mode's answer for a group
/// that is not on the host yet.
fn owner_gid(owner: &HomeOwner, name: &str) -> Result<Option<u64>, String> {
    match owner {
        HomeOwner::Number(gid) => Ok(Some(*gid)),
        HomeOwner::SameName => Ok(group_entry(name)?.map(|(_, gid)| gid)),
    }
}

/// The ownership the home's repair finds, before it changes anything:
/// what a later failure restores — `None` where the home is not there
/// yet, so the repair creates it and the rollback takes it away. Any
/// other lookup error fails the repair itself before anything mutates,
/// and the rollback the failure triggers covers the mutations before
/// this one.
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

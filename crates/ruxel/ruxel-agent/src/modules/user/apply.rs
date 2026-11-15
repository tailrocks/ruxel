//! The changes an existing account's step runs, in the order they run:
//! the private group the step claims, the home's repair, and the
//! account's own change.

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
    /// or nothing, where the repair created the directory — removed.
    Home {
        path: String,
        prior: Option<(u64, u64)>,
    },
}

/// The changes the existing account's step runs, in the order the plan
/// made them. The home's repair runs before the account's own change:
/// it is the preflight of that change, so a repair that fails leaves
/// the account as it was. A mutation that fails takes back what the
/// earlier ones did — the created group, the repaired home — so a
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
                    // directory away where the repair created it. Check
                    // mode mutates nothing, so there is nothing to take
                    // back.
                    let prior = if check_mode {
                        None
                    } else {
                        prior_owner(&home.path)?
                    };
                    let repaired = ensure_home(&home.path, home.uid, gid, check_mode)?;
                    if !check_mode {
                        completed.push(Completed::Home {
                            path: home.path,
                            prior,
                        });
                    }
                    Ok(repaired)
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

/// What the earlier mutations did, taken back in reverse order: the
/// private group the step created is deleted, and the home's repair is
/// reversed — the ownership it found restored, the directory it created
/// removed. A rollback that fails itself is reported with the error
/// that caused it: the host keeps the partial change, and the task
/// says so instead of reporting a clean refusal.
fn take_back(completed: &[Completed], run_cmd: &Runner, error: String) -> String {
    let mut failures = Vec::new();
    for done in completed.iter().rev() {
        let undone = match done {
            Completed::Group(group) => run_cmd("groupdel", &[group]).map_err(|rollback| {
                format!("the group {group} could not be taken back: {rollback}")
            }),
            Completed::Home { path, prior } => super::home::rollback_home(path, *prior)
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

/// The rollback of a failed change: the mutations a later step's
/// failure takes back, over canned commands and scratch directories —
/// no host account is touched.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The identity the test process owns: a probe file of it, so no
    /// ownership change is needed where the repair targets it.
    fn own_identity(scenario: &str) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let probe = std::env::temp_dir().join(format!(
            "ruxel-rollback-probe-{scenario}-{}",
            std::process::id()
        ));
        std::fs::write(&probe, b"").unwrap();
        let meta = std::fs::metadata(&probe).unwrap();
        let identity = (u64::from(meta.uid()), u64::from(meta.gid()));
        std::fs::remove_file(&probe).unwrap();
        identity
    }

    fn home_mutation(path: &std::path::Path, uid: u64, gid: u64) -> Mutation {
        Mutation::Home(Home {
            path: path.to_str().unwrap().to_string(),
            uid,
            owner: HomeOwner::Number(gid),
        })
    }

    #[test]
    fn a_failed_home_repair_takes_back_the_created_group() {
        // The group's own creation runs before the home's repair, and
        // the repair can fail — an immutable directory, a read-only
        // parent, a path under a regular file. The group the step
        // created must not stay behind when the task reports failure.
        let file = std::env::temp_dir().join(format!("ruxel-rollback-file-{}", std::process::id()));
        std::fs::write(&file, b"not a directory").unwrap();
        let home = file.join("home");
        let group = format!("ruxel-rollback-group-{}", std::process::id());
        let mutations = vec![
            Mutation::GroupAdd(group.clone()),
            home_mutation(&home, 1500, 1500),
        ];
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let error = run("ruxel-rollback-alice", mutations, false, &{
            let calls = std::rc::Rc::clone(&calls);
            move |cmd: &str, args: &[&str]| {
                calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
                Ok(())
            }
        })
        .expect_err("the home's repair must fail the task");
        assert!(error.contains("home"), "{error}");
        assert_eq!(
            calls.borrow().clone(),
            vec![format!("groupadd {group}"), format!("groupdel {group}")],
            "the created group is taken back: {error}"
        );
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn a_failed_account_change_takes_back_the_home_and_the_group() {
        // The home's repair runs before the account's own change, and
        // the change can fail after the repair created the directory.
        // The created directory and the created group must both go back
        // with the failure.
        let base = std::env::temp_dir().join(format!("ruxel-rollback-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let home = base.join("alice");
        let (uid, gid) = own_identity("created");
        let group = format!("ruxel-rollback-group-{}", std::process::id());
        let mutations = vec![
            Mutation::GroupAdd(group.clone()),
            home_mutation(&home, uid, gid),
            Mutation::UserMod(vec!["-s".to_string(), "/bin/sh".to_string()]),
        ];
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let error = run("ruxel-rollback-alice", mutations, false, &{
            let calls = std::rc::Rc::clone(&calls);
            move |cmd: &str, args: &[&str]| {
                calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
                if cmd == "usermod" {
                    return Err("injected usermod failure".to_string());
                }
                Ok(())
            }
        })
        .expect_err("the account's change must fail the task");
        assert!(error.contains("injected usermod failure"), "{error}");
        assert_eq!(
            calls.borrow().clone(),
            vec![
                format!("groupadd {group}"),
                "usermod -s /bin/sh ruxel-rollback-alice".to_string(),
                format!("groupdel {group}"),
            ],
            "the repair is reversed before the group: {error}"
        );
        assert!(
            !home.exists(),
            "the directory the repair created is taken back"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_failed_repair_of_an_existing_home_restores_its_ownership() {
        // The repair of a home that exists keeps the directory and only
        // changes its owner; the rollback restores the ownership the
        // repair found. The directory already belongs to the test
        // process, so no ownership change is needed unprivileged — the
        // rollback runs the same path and keeps the directory.
        let base = std::env::temp_dir().join(format!("ruxel-rollback-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let (uid, gid) = own_identity("kept");
        let group = format!("ruxel-rollback-group-{}", std::process::id());
        let mutations = vec![
            Mutation::GroupAdd(group.clone()),
            home_mutation(&base, uid, gid),
            Mutation::UserMod(vec!["-s".to_string(), "/bin/sh".to_string()]),
        ];
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let error = run("ruxel-rollback-alice", mutations, false, &{
            let calls = std::rc::Rc::clone(&calls);
            move |cmd: &str, args: &[&str]| {
                calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
                if cmd == "usermod" {
                    return Err("injected usermod failure".to_string());
                }
                Ok(())
            }
        })
        .expect_err("the account's change must fail the task");
        assert!(error.contains("injected usermod failure"), "{error}");
        assert_eq!(
            calls.borrow().clone(),
            vec![
                format!("groupadd {group}"),
                "usermod -s /bin/sh ruxel-rollback-alice".to_string(),
                format!("groupdel {group}"),
            ],
            "the existing home is restored before the group: {error}"
        );
        assert!(base.is_dir(), "the existing home is kept, only restored");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_failed_rollback_is_reported_with_the_original_error() {
        // The rollback itself can fail — the group is already gone, the
        // directory is held open. The task then reports both errors: the
        // host keeps the partial change, and the report says so instead
        // of claiming a clean refusal.
        let file =
            std::env::temp_dir().join(format!("ruxel-rollback-doomed-{}", std::process::id()));
        std::fs::write(&file, b"not a directory").unwrap();
        let home = file.join("home");
        let group = format!("ruxel-rollback-group-{}", std::process::id());
        let mutations = vec![
            Mutation::GroupAdd(group.clone()),
            home_mutation(&home, 1500, 1500),
        ];
        let error = run("ruxel-rollback-alice", mutations, false, &|cmd, _| {
            if cmd == "groupdel" {
                return Err("injected groupdel failure".to_string());
            }
            Ok(())
        })
        .expect_err("the home's repair must fail the task");
        assert!(error.contains("home"), "{error}");
        assert!(
            error.contains("could not be taken back")
                && error.contains("injected groupdel failure"),
            "the rollback failure is reported with the original error: {error}"
        );
        std::fs::remove_file(&file).unwrap();
    }
}

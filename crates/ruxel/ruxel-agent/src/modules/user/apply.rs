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

/// The changes the existing account's step runs, in the order the plan
/// made them. The home's repair runs before the account's own change:
/// it is the preflight of that change, so a repair that fails leaves
/// the account as it was. Returns whether the home's repair changed
/// (or would change in check mode) anything.
pub(super) fn run(
    name: &str,
    mutations: Vec<Mutation>,
    check_mode: bool,
    run_cmd: &Runner,
) -> Result<bool, String> {
    let mut changed = false;
    for mutation in mutations {
        match mutation {
            Mutation::GroupAdd(group) => {
                if !check_mode {
                    run_cmd("groupadd", &[&group])?;
                }
            }
            Mutation::Home(home) => {
                if let Some(gid) = owner_gid(&home.owner, name)? {
                    changed |= ensure_home(&home.path, home.uid, gid, check_mode)?;
                }
            }
            Mutation::UserMod(mut args) => {
                if !check_mode {
                    args.push(name.to_string());
                    let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
                    run_cmd("usermod", &arguments)?;
                }
            }
        }
    }
    Ok(changed)
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

//! The plan's own record of the identities it has declared so far: the
//! check-mode counterpart of the host's passwd and group databases —
//! check mode runs nothing, so every guard that resolves an identity
//! reads what the plan's own earlier tasks would have created.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// What the plan's step requests for an identity it creates: the
/// declared number, or presence without one (`useradd` allocates one).
#[derive(Clone, Copy)]
pub(crate) enum Requested {
    Number(u32),
    Unnumbered,
}

/// The primary group the plan gives an account: the group's name and
/// the number the plan knows for it. The number is `None` where the
/// group is not on the host yet — check mode runs no `groupadd`, so
/// the number the allocator draws is unknown until the apply draws
/// it.
#[derive(Clone)]
pub(super) struct PlannedPrimary {
    pub(super) name: String,
    pub(super) gid: Option<u64>,
}

impl PlannedPrimary {
    /// Whether the account still holds the group of the number `gid` —
    /// the group named `group` — as its primary group under the plan.
    /// The number the plan knows decides, and the name where the plan
    /// does not know the number yet: the group is not on the host, so
    /// the allocator draws its number at the apply.
    pub(super) fn holds(&self, gid: u64, group: &str) -> bool {
        match self.gid {
            Some(planned_gid) => planned_gid == gid,
            None => self.name == group,
        }
    }
}

thread_local! {
    static GROUPS: RefCell<BTreeMap<String, Option<u32>>> = const { RefCell::new(BTreeMap::new()) };
    static USERS: RefCell<BTreeMap<String, Option<u32>>> = const { RefCell::new(BTreeMap::new()) };
    static HOMES: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    static REMOVED_ACCOUNTS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static REMOVED_GROUPS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static PRIMARY_GROUPS: RefCell<BTreeMap<String, PlannedPrimary>> = const { RefCell::new(BTreeMap::new()) };
    static SUPPLEMENTARY: RefCell<BTreeMap<String, Vec<String>>> = const { RefCell::new(BTreeMap::new()) };
}

pub(super) fn note(name: &str, requested: Option<u32>) {
    GROUPS.with(|groups| {
        let mut groups = groups.borrow_mut();
        // A present step that states no number changes no number:
        // an earlier step's explicit one stays the plan's effective
        // id, what a later removal's declaration is verified
        // against.
        let requested = requested.or_else(|| groups.get(name).copied().flatten());
        groups.insert(name.to_string(), requested);
    });
}

pub(super) fn forget(name: &str) {
    GROUPS.with(|groups| {
        groups.borrow_mut().remove(name);
    });
}

/// What the plan's group step requested; `None`: undeclared.
pub(super) fn group_requested(name: &str) -> Option<Requested> {
    GROUPS.with(|groups| {
        groups
            .borrow()
            .get(name)
            .map(|r| r.map_or(Requested::Unnumbered, Requested::Number))
    })
}

pub(super) fn note_user(name: &str, requested: Option<u32>) {
    USERS.with(|users| {
        let mut users = users.borrow_mut();
        // The account counterpart: a present step without a uid
        // renumbers nothing.
        let requested = requested.or_else(|| users.get(name).copied().flatten());
        users.insert(name.to_string(), requested);
    });
}

pub(super) fn forget_user(name: &str) {
    USERS.with(|users| {
        users.borrow_mut().remove(name);
    });
}

/// The plan's record of the account's home: the path the account's
/// passwd entry carries under the plan — what a removal verifies its
/// declaration against, since `userdel -r` deletes that home and no
/// other.
pub(super) fn note_home(account: &str, home: &str) {
    HOMES.with(|homes| {
        homes
            .borrow_mut()
            .insert(account.to_string(), home.to_string());
    });
}

/// The account's planned home, where a task of the plan gave it one.
pub(super) fn home(account: &str) -> Option<String> {
    HOMES.with(|homes| homes.borrow().get(account).cloned())
}

/// The record's counterpart when the plan removes the account.
pub(super) fn forget_home(account: &str) {
    HOMES.with(|homes| {
        homes.borrow_mut().remove(account);
    });
}

/// The plan removed the account and declares it present again: the
/// apply takes the account away and creates it anew, so the number
/// the account holds is the allocator's to draw — the stale live entry
/// must not stand in for it.
pub(crate) fn user_recreated(name: &str) -> bool {
    account_removed(name) && user_requested(name).is_some()
}

/// The group counterpart: the plan removed the group and declares it
/// present again, so its number is the allocator's to draw.
pub(crate) fn group_recreated(name: &str) -> bool {
    group_removed(name) && group_requested(name).is_some()
}

/// What the plan's account step requested; `None`: undeclared.
pub(super) fn user_requested(name: &str) -> Option<Requested> {
    USERS.with(|users| {
        users
            .borrow()
            .get(name)
            .map(|r| r.map_or(Requested::Unnumbered, Requested::Number))
    })
}

/// The number the plan has given the account: the number a task of the
/// plan declared, `None` where no task declared one.
pub(super) fn uid(name: &str) -> Option<u64> {
    match user_requested(name) {
        Some(Requested::Number(number)) => Some(u64::from(number)),
        _ => None,
    }
}

/// The plan's record of the account it has removed: the number the
/// account held is free for whatever the plan claims next, although
/// the host's database still lists it while check mode runs.
pub(super) fn note_removed_account(name: &str) {
    REMOVED_ACCOUNTS.with(|accounts| {
        accounts.borrow_mut().insert(name.to_string());
    });
}

pub(super) fn account_removed(name: &str) -> bool {
    REMOVED_ACCOUNTS.with(|accounts| accounts.borrow().contains(name))
}

/// The plan's record of a group it has removed: the number the group
/// held is free for whatever the plan claims next.
pub(super) fn note_removed_group(name: &str) {
    REMOVED_GROUPS.with(|groups| {
        groups.borrow_mut().insert(name.to_string());
    });
}

pub(super) fn group_removed(name: &str) -> bool {
    REMOVED_GROUPS.with(|groups| groups.borrow().contains(name))
}

/// Whether the plan's steps move the account off the number it
/// holds on the host: the record carries the requested number, and
/// one that differs from the live entry frees it — `usermod -u`
/// sets the new uid and the old one is the next `useradd -u`'s.
pub(super) fn user_renumbered(name: &str, uid: u64) -> bool {
    matches!(user_requested(name), Some(Requested::Number(n)) if u64::from(n) != uid)
}

/// The group counterpart: `groupmod -g` frees the old gid the same
/// way.
pub(super) fn group_renumbered(name: &str, gid: u64) -> bool {
    matches!(group_requested(name), Some(Requested::Number(n)) if u64::from(n) != gid)
}

/// The plan's record of the account's primary group: the group a task
/// of the plan gives the account, which the host's database does not
/// show while check mode runs.
pub(super) fn note_primary(account: &str, primary: PlannedPrimary) {
    PRIMARY_GROUPS.with(|groups| {
        groups.borrow_mut().insert(account.to_string(), primary);
    });
}

/// The account's planned primary group, where a task of the plan gave
/// it one.
pub(super) fn primary(account: &str) -> Option<PlannedPrimary> {
    PRIMARY_GROUPS.with(|groups| groups.borrow().get(account).cloned())
}

/// The record's counterpart when the plan removes the account.
pub(super) fn forget_primary(account: &str) {
    PRIMARY_GROUPS.with(|groups| {
        groups.borrow_mut().remove(account);
    });
}

/// The plan's record of the account's supplementary groups: the set a
/// task of the plan gives the account, which the host's group
/// database does not show while check mode runs.
pub(super) fn note_supplementary(account: &str, groups: Vec<String>) {
    SUPPLEMENTARY.with(|sets| {
        sets.borrow_mut().insert(account.to_string(), groups);
    });
}

/// The account's planned supplementary set, where a task of the plan
/// gave it one.
pub(super) fn supplementary(account: &str) -> Option<Vec<String>> {
    SUPPLEMENTARY.with(|sets| sets.borrow().get(account).cloned())
}

/// The record's counterpart when the plan removes the account.
pub(super) fn forget_supplementary(account: &str) {
    SUPPLEMENTARY.with(|sets| {
        sets.borrow_mut().remove(account);
    });
}

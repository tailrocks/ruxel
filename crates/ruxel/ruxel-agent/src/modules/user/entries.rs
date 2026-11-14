//! The host's account and group entries: the passwd and group content
//! every account and group operation reads before it touches anything.

#[derive(Default, Clone)]
pub(super) struct PasswdEntry {
    pub(super) uid: u64,
    pub(super) gid: u64,
    pub(super) comment: String,
    pub(super) home: String,
    pub(super) shell: String,
}

pub(super) fn group_entry(name: &str) -> Result<Option<(String, u64)>, String> {
    let content = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    group_entry_from(&content, name)
}

pub(super) fn group_entry_from(content: &str, name: &str) -> Result<Option<(String, u64)>, String> {
    for line in content.lines() {
        let mut f = line.split(':');
        if f.next() == Some(name) {
            let gid = f
                .nth(1)
                .and_then(|g| g.parse().ok())
                .ok_or("malformed group line")?;
            return Ok(Some((name.to_string(), gid)));
        }
    }
    Ok(None)
}

pub(super) fn passwd_entry(name: &str) -> Result<Option<PasswdEntry>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    passwd_entry_from(&passwd, name)
}

pub(super) fn passwd_entry_from(passwd: &str, name: &str) -> Result<Option<PasswdEntry>, String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&name) && f.len() >= 7 {
            return Ok(Some(PasswdEntry {
                uid: f[2].parse().map_err(|_| "bad uid")?,
                gid: f[3].parse().map_err(|_| "bad gid")?,
                comment: f[4].to_string(),
                home: f[5].to_string(),
                shell: f[6].to_string(),
            }));
        }
    }
    Ok(None)
}

/// The account that holds `uid` in the passwd content, apart from
/// `name` itself: the account's own number is no collision with
/// itself. `useradd -u` and `usermod -u` refuse a number another
/// account holds, so the request is checked against the live database
/// before anything runs.
pub(super) fn uid_holder_from(passwd: &str, uid: u64, name: &str) -> Option<String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(uid) && f.first() != Some(&name) {
            return Some(f[0].to_string());
        }
    }
    None
}

/// The group that holds `gid` in the group content, apart from `name`
/// itself. `groupadd -g` and `groupmod -g` refuse a number another
/// group holds.
pub(super) fn gid_holder_from(groups: &str, gid: u64, name: &str) -> Option<String> {
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(gid) && f.first() != Some(&name) {
            return Some(f[0].to_string());
        }
    }
    None
}

pub(super) fn supplementary_groups(name: &str) -> Result<Vec<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(supplementary_groups_from(&groups, name))
}

pub(super) fn supplementary_groups_from(groups: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[3].split(',').any(|m| m == name) {
            out.push(f[0].to_string());
        }
    }
    out
}

pub(super) fn groups_need_update(current: &[String], want: &[String], append: bool) -> bool {
    let missing = want.iter().any(|group| !current.contains(group));
    (append && missing)
        || (!append
            && (current.len() != want.len() || want.iter().any(|group| !current.contains(group))))
}

/// What the plan's step requests for an identity it creates: the
/// declared number, or presence without one (`useradd` allocates one).
#[derive(Clone, Copy)]
pub(crate) enum Requested {
    Number(u32),
    Unnumbered,
}

/// The identities a plan has declared present so far: the check-mode
/// counterpart of the host's passwd and group databases — check mode
/// runs nothing, so `planned_gid` and the file module's ownership
/// resolution read what the plan's own earlier tasks would have
/// created. The record carries the requested number where the step
/// declared one; an identity no task provides is refused exactly as
/// apply refuses it.
mod planned {
    use super::Requested;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    thread_local! {
        static GROUPS: RefCell<BTreeMap<String, Option<u32>>> = const { RefCell::new(BTreeMap::new()) };
        static USERS: RefCell<BTreeMap<String, Option<u32>>> = const { RefCell::new(BTreeMap::new()) };
        static REMOVED_ACCOUNTS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
        static REMOVED_GROUPS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
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

    /// What the plan's account step requested; `None`: undeclared.
    pub(super) fn user_requested(name: &str) -> Option<Requested> {
        USERS.with(|users| {
            users
                .borrow()
                .get(name)
                .map(|r| r.map_or(Requested::Unnumbered, Requested::Number))
        })
    }

    /// The plan's record of an account it has removed: the number the
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

    /// The plan's record of a group it has removed: the number the
    /// group held is free for whatever the plan claims next.
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
}

/// The plan's record of a present group — or an account's private
/// group — for whatever resolves the group next; carries the requested
/// gid where the step declared one.
pub(super) fn note_planned_group(name: &str, requested: Option<u32>) {
    planned::note(name, requested);
}

/// What the plan's group step requested; `None`: undeclared.
pub(crate) fn planned_group(name: &str) -> Option<Requested> {
    planned::group_requested(name)
}

/// The plan's record of a present account: what the file module's
/// ownership resolves against in check mode, which runs no `useradd`.
/// It carries the requested uid where the step declared one.
pub(super) fn note_planned_user(name: &str, requested: Option<u32>) {
    planned::note_user(name, requested);
}

/// The record's counterpart when the plan removes the account.
pub(super) fn forget_planned_user(name: &str) {
    planned::forget_user(name);
}

/// What the plan's account step requested; `None`: undeclared.
pub(crate) fn planned_user(name: &str) -> Option<Requested> {
    planned::user_requested(name)
}

/// The record's counterpart when the plan removes the group.
pub(super) fn forget_planned_group(name: &str) {
    planned::forget(name);
}

/// The plan's record of the account it has removed: the number the
/// account held is free for whatever the plan claims next. Check mode
/// runs no `userdel`, so the host's database still lists the account
/// while the plan knows it is gone.
pub(super) fn note_removed_account(name: &str) {
    planned::note_removed_account(name);
}

/// Whether the plan has removed the account: a holder the plan took
/// away holds no number, so it is no collision.
pub(super) fn account_removed(name: &str) -> bool {
    planned::account_removed(name)
}

/// The plan's record of the group it has removed: the number the group
/// held is free for whatever the plan claims next.
pub(super) fn note_removed_group(name: &str) {
    planned::note_removed_group(name);
}

/// Whether the plan has removed the group: a holder the plan took away
/// holds no number, so it is no collision.
pub(super) fn group_removed(name: &str) -> bool {
    planned::group_removed(name)
}

/// Whether the plan's own earlier steps free the number the account
/// holds on the host: the removal takes the account away, and a
/// renumbering frees the old number before the change — `useradd -u`
/// and `usermod -u` refuse a number another account holds, and this
/// one holds it no longer.
pub(crate) fn account_freed(name: &str, uid: u64) -> bool {
    planned::account_removed(name) || planned::user_renumbered(name, uid)
}

/// The group counterpart: a removed or renumbered group holds no
/// number.
pub(crate) fn group_freed(name: &str, gid: u64) -> bool {
    planned::group_removed(name) || planned::group_renumbered(name, gid)
}

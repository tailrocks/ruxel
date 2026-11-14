//! The host's account and group entries: the passwd and group content
//! every account and group operation reads before it touches anything.

use super::plan;
pub(crate) use super::plan::Requested;

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

/// The accounts that hold `uid` in the passwd content, apart from
/// `name` itself: the account's own number is no collision with
/// itself. Every holder is returned, not just the first: a host with
/// duplicate numbers has more than one, and the one the plan has not
/// freed is the collision `useradd -u` and `usermod -u` refuse.
pub(super) fn uid_holders_from(passwd: &str, uid: u64, name: &str) -> Vec<String> {
    let mut holders = Vec::new();
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(uid) && f.first() != Some(&name) {
            holders.push(f[0].to_string());
        }
    }
    holders
}

/// The groups that hold `gid` in the group content, apart from `name`
/// itself. `groupadd -g` and `groupmod -g` refuse a number another
/// group holds — every holder is returned, for the same reason the
/// account scan returns every holder.
pub(super) fn gid_holders_from(groups: &str, gid: u64, name: &str) -> Vec<String> {
    let mut holders = Vec::new();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(gid) && f.first() != Some(&name) {
            holders.push(f[0].to_string());
        }
    }
    holders
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

/// The plan's record of a present group — or an account's private
/// group — for whatever resolves the group next; carries the requested
/// gid where the step declared one.
pub(super) fn note_planned_group(name: &str, requested: Option<u32>) {
    plan::note(name, requested);
}

/// What the plan's group step requested; `None`: undeclared.
pub(crate) fn planned_group(name: &str) -> Option<Requested> {
    plan::group_requested(name)
}

/// The plan's record of a present account: what the file module's
/// ownership resolves against in check mode, which runs no `useradd`.
/// It carries the requested uid where the step declared one.
pub(super) fn note_planned_user(name: &str, requested: Option<u32>) {
    plan::note_user(name, requested);
}

/// The record's counterpart when the plan removes the account.
pub(super) fn forget_planned_user(name: &str) {
    plan::forget_user(name);
}

/// What the plan's account step requested; `None`: undeclared.
pub(crate) fn planned_user(name: &str) -> Option<Requested> {
    plan::user_requested(name)
}

/// The number the plan has given the account: the number a task of the
/// plan declared, `None` where no task declared one.
pub(super) fn planned_uid(name: &str) -> Option<u64> {
    plan::uid(name)
}

/// The record's counterpart when the plan removes the group.
pub(super) fn forget_planned_group(name: &str) {
    plan::forget(name);
}

/// The plan's record of the account it has removed: the number the
/// account held is free for whatever the plan claims next. Check mode
/// runs no `userdel`, so the host's database still lists the account
/// while the plan knows it is gone.
pub(super) fn note_removed_account(name: &str) {
    plan::note_removed_account(name);
}

/// Whether the plan has removed the account: a holder the plan took
/// away holds no number, so it is no collision.
pub(super) fn account_removed(name: &str) -> bool {
    plan::account_removed(name)
}

/// The plan's record of the group it has removed: the number the group
/// held is free for whatever the plan claims next.
pub(super) fn note_removed_group(name: &str) {
    plan::note_removed_group(name);
}

/// Whether the plan has removed the group: a holder the plan took away
/// holds no number, so it is no collision.
pub(super) fn group_removed(name: &str) -> bool {
    plan::group_removed(name)
}

/// Whether the plan's own earlier steps free the number the account
/// holds on the host: the removal takes the account away, and a
/// renumbering frees the old number before the change — `useradd -u`
/// and `usermod -u` refuse a number another account holds, and this
/// one holds it no longer.
pub(crate) fn account_freed(name: &str, uid: u64) -> bool {
    plan::account_removed(name) || plan::user_renumbered(name, uid)
}

/// The group counterpart: a removed or renumbered group holds no
/// number.
pub(crate) fn group_freed(name: &str, gid: u64) -> bool {
    plan::group_removed(name) || plan::group_renumbered(name, gid)
}

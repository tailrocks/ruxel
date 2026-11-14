//! The requested ownership of a file, resolved to ids: the plan's own
//! explicitly requested numbers win over the stale live entries, the
//! check-mode counterpart of the host's passwd and group databases.

use serde_json::{Map, Value};

use super::str_param;

/// The requested ownership of a file, resolved to ids. An identity the
/// plan creates without stating a number has no id to resolve to before
/// the apply allocates one: the request is pending — an ownership
/// change to report, never "no owner constraint".
pub(crate) struct Ownership {
    pub(super) uid: Option<u32>,
    pub(super) gid: Option<u32>,
    pub(super) uid_pending: bool,
    pub(super) gid_pending: bool,
}

/// Resolves the requested owner and group of `obj` to ids.
///
/// The ids are resolved once and reused by the caller: a change to the
/// passwd or group file between the resolution and the use of the ids
/// cannot fail or mis-apply a publication that already succeeded.
pub(crate) fn resolve_ownership(obj: &Map<String, Value>) -> Result<Ownership, String> {
    let (uid, uid_pending) = match str_param(obj, "owner").map(resolve_uid).transpose()? {
        Some((id, pending)) => (id, pending),
        None => (None, false),
    };
    let (gid, gid_pending) = match str_param(obj, "group").map(resolve_gid).transpose()? {
        Some((id, pending)) => (id, pending),
        None => (None, false),
    };
    Ok(Ownership {
        uid,
        gid,
        uid_pending,
        gid_pending,
    })
}

// The number of ownership resolutions of the current thread.
//
// The counter is a test aid: a module that resolves the requested
// ownership once must not resolve it again.
#[cfg(test)]
thread_local! {
    pub(crate) static OWNERSHIP_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts one ownership resolution of the current thread.
#[cfg(test)]
fn count_ownership_resolution() {
    OWNERSHIP_RESOLUTIONS.with(|count| count.set(count.get() + 1));
}

fn resolve_uid(owner: &str) -> Result<(Option<u32>, bool), String> {
    #[cfg(test)]
    count_ownership_resolution();
    if let Ok(uid) = owner.parse::<u32>() {
        return Ok((Some(uid), false));
    }
    // The plan's explicitly requested number consults before the live
    // entry is accepted: an earlier `User` task renumbers an existing
    // account (`usermod -u`) and check mode runs nothing, so the passwd
    // file still holds the stale number — accepting it would report the
    // file unchanged while the apply runs `usermod -u`, resolves the
    // account to the new number and chowns the file.
    match super::user::planned_user(owner) {
        Some(super::user::Requested::Number(uid)) => return Ok((Some(uid), false)),
        // The plan removed the account and declares it present again:
        // the apply takes the account away and creates it anew, so the
        // number is the allocator's to draw — the stale live entry must
        // not stand in for it, or a file the plan owns with that
        // account is reported unchanged while the apply chowns it to
        // the number the allocator drew. The requested ownership is
        // PENDING, like any other unnumbered identity.
        Some(super::user::Requested::Unnumbered) if super::user::user_recreated(owner) => {
            return Ok((None, true));
        }
        _ => {}
    }
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    match resolve_uid_from(&passwd, owner) {
        Ok(uid) => Ok((Some(uid), false)),
        // A name the plan's own earlier steps create does not resolve
        // in check mode: the record carries it (the requested uid where
        // the step declared one), and only a name no task of the plan
        // provides is refused the way apply refuses it. An unnumbered
        // creation gets its id from the allocator at apply — unknown
        // now, but the requested ownership is PENDING, not absent: the
        // apply creates the account and chowns the file to the
        // allocated id, so an otherwise-correct path must report the
        // change, never claim no owner constraint.
        Err(_) => match super::user::planned_user(owner) {
            Some(super::user::Requested::Unnumbered) => Ok((None, true)),
            _ => Err(format!("user {owner:?} not found")),
        },
    }
}

pub(crate) fn resolve_uid_from(passwd: &str, owner: &str) -> Result<u32, String> {
    for line in passwd.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(owner) {
            let uid = fields.nth(1).ok_or("malformed passwd line")?;
            return uid.parse().map_err(|_| "malformed uid".to_string());
        }
    }
    Err(format!("user {owner:?} not found"))
}

fn resolve_gid(group: &str) -> Result<(Option<u32>, bool), String> {
    #[cfg(test)]
    count_ownership_resolution();
    if let Ok(gid) = group.parse::<u32>() {
        return Ok((Some(gid), false));
    }
    // The gid counterpart: an earlier `Group` task renumbers an
    // existing group (`groupmod -g`) and check mode runs nothing, so
    // the planned number is what the apply ends up with.
    match super::user::planned_group(group) {
        Some(super::user::Requested::Number(gid)) => return Ok((Some(gid), false)),
        // The group counterpart of the recreated account: the plan
        // removed the group and declares it present again, so its
        // number is the allocator's to draw and the stale live entry
        // must not stand in for it.
        Some(super::user::Requested::Unnumbered) if super::user::group_recreated(group) => {
            return Ok((None, true));
        }
        _ => {}
    }
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    match resolve_gid_from(&groups, group) {
        Ok(gid) => Ok((Some(gid), false)),
        // The same for a group the plan's own earlier steps create: an
        // unnumbered creation leaves the requested ownership pending,
        // not absent.
        Err(_) => match super::user::planned_group(group) {
            Some(super::user::Requested::Unnumbered) => Ok((None, true)),
            _ => Err(format!("group {group:?} not found")),
        },
    }
}

pub(crate) fn resolve_gid_from(groups: &str, group: &str) -> Result<u32, String> {
    for line in groups.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(group) {
            let gid = fields.nth(1).ok_or("malformed group line")?;
            return gid.parse().map_err(|_| "malformed gid".to_string());
        }
    }
    Err(format!("group {group:?} not found"))
}

#[cfg(test)]
#[path = "tests/ownership.rs"]
mod tests;

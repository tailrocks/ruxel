//! The host's to keep: the guards that verify live entries before the
//! account and group utilities run.

use super::PasswdEntry;

/// Whether the agent may change the identity of the account whose host
/// entry is `entry`: the live uid-0 account is the superuser's whatever
/// it is named, and moving it off uid 0 — or off its primary group —
/// strands every file it owns.
pub(super) fn identity_change_allowed(name: &str, entry: &PasswdEntry) -> Result<(), String> {
    if entry.uid == 0 {
        return Err(format!(
            "user: refusing to change the identity of {name}: the superuser's account (uid 0) is the host's to keep"
        ));
    }
    Ok(())
}

/// The gid counterpart: the live gid-0 group is the root group whatever
/// it is named.
pub(super) fn group_change_allowed(name: &str, host_gid: u64) -> Result<(), String> {
    if host_gid == 0 {
        return Err(format!(
            "group: refusing to change the gid of {name}: the root group (gid 0) is the host's to keep"
        ));
    }
    Ok(())
}

/// Whether the agent may remove the account whose host entry is `entry`.
/// The deletion verifies the host entry rather than trusting the workflow
/// value: the superuser's account and a system account below UID 1000 are
/// the host's to keep whatever the workflow states, and a declared uid
/// must match the host's — `user = "daemon"; uid = 1000` must not remove
/// an account whose real UID is below 1000.
pub(super) fn removal_allowed(
    name: &str,
    declared: Option<u64>,
    entry: &PasswdEntry,
) -> Result<(), String> {
    if entry.uid == 0 {
        return Err(format!(
            "user: refusing to remove {name}: the superuser's account (uid 0) is the host's to keep"
        ));
    }
    if entry.uid < 1000 {
        return Err(format!(
            "user: refusing to remove {name}: the host account is a system account (uid {}), which the host keeps",
            entry.uid
        ));
    }
    if let Some(want) = declared
        && want != entry.uid
    {
        return Err(format!(
            "user: refusing to remove {name}: the host uid {} is not the declared {want}",
            entry.uid
        ));
    }
    Ok(())
}

/// Whether the agent may remove the group of host gid `host_gid`. The
/// root group (GID 0) is the host's to keep whatever it is named, and a
/// declared gid must match the host's: the host entry, not the workflow
/// value, is what deletion acts on.
pub(super) fn group_removal_allowed(
    name: &str,
    declared: Option<u64>,
    host_gid: u64,
) -> Result<(), String> {
    if host_gid == 0 {
        return Err(format!(
            "group: refusing to remove {name}: the root group (gid 0) is the host's to keep"
        ));
    }
    if let Some(want) = declared
        && want != host_gid
    {
        return Err(format!(
            "group: refusing to remove {name}: the host gid {host_gid} is not the declared {want}"
        ));
    }
    Ok(())
}

/// `userdel -r` deletes the account's live home and mail spool whatever
/// the workflow names, so a recursive removal verifies the declared home
/// against the live entry: the lowering reserves the declared path only,
/// and a same-stage file beneath the live home would be deleted though
/// no check ever reserved it.
pub(super) fn removal_home_allowed(
    name: &str,
    home: Option<&str>,
    remove: bool,
    live_home: &str,
) -> Result<(), String> {
    if remove && home != Some(live_home) {
        return Err(format!(
            "user: refusing to remove {name} with removeHome: the declared home {home:?} is not the live home {live_home}, which userdel -r would delete"
        ));
    }
    Ok(())
}

/// The name of the account's live primary group, resolved through the
/// group database: the passwd entry carries the gid only.
pub(super) fn primary_group_name(gid: u64) -> Result<Option<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(group_name_of_gid_from(&groups, gid))
}

pub(super) fn group_name_of_gid_from(groups: &str, gid: u64) -> Option<String> {
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(gid) {
            return Some(f[0].to_string());
        }
    }
    None
}

/// Whether the removal's declared primary group is the live one. The
/// lowering derives the removal's ordering from the declared primary
/// group — a `Group` removal that would strand it must list the account
/// removal first — which only holds where the declaration matches the
/// host: an account originally created with another primary group makes
/// the private-group default wrong, so the removal refuses and asks for
/// the real group to be stated, before `userdel` mutates anything.
pub(super) fn removal_primary_group_allowed(
    name: &str,
    declared: Option<&str>,
    live: Option<&str>,
) -> Result<(), String> {
    let want = declared.unwrap_or(name);
    match live {
        Some(actual) if actual == want => Ok(()),
        Some(actual) => Err(format!(
            "user: refusing to remove {name}: the live primary group {actual} is not the declared {want}; state the account's primary group (group = \"{actual}\") to remove it"
        )),
        None => Ok(()),
    }
}

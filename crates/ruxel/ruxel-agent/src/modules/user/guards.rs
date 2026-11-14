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
/// value, is what deletion acts on. A group a live account holds as its
/// primary group is the account's to keep: `groupdel` refuses to delete
/// a primary group without `--force`, which this removal does not pass,
/// so the refusal comes before the utility runs — in check mode as much
/// as in apply.
pub(super) fn group_removal_allowed(
    name: &str,
    declared: Option<u64>,
    host_gid: u64,
    primary_holder: Option<&str>,
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
    if let Some(holder) = primary_holder {
        return Err(format!(
            "group: refusing to remove {name}: the account {holder} has it as its primary group, and groupdel needs --force to remove it"
        ));
    }
    Ok(())
}

/// The live account that holds `gid` as its primary group: the passwd
/// database's entry with that number in its group field. The account is
/// the group's to keep — `groupdel` refuses to delete a primary group
/// without `--force` — so the removal reads it before it runs. An
/// account the plan has already removed holds no group: check mode
/// runs no `userdel`, so the passwd file still lists the account while
/// the apply has already taken it away — the scan skips it and keeps
/// looking for a remaining holder. An account the plan gives another
/// primary group holds this one no longer: the apply moves the
/// account first (`usermod -g`), and the scan follows the plan's own
/// record rather than the unchanged passwd file.
pub(super) fn primary_group_holder(gid: u64, group: &str) -> Result<Option<String>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    Ok(primary_group_holder_from(&passwd, gid, group))
}

/// The passwd counterpart of [`primary_group_holder`], over canned
/// content. The accounts the plan has removed are no holders, and the
/// accounts the plan gives another primary group hold this one no
/// longer: the scan reads the plan's own record, so it skips the
/// accounts the plan moved away and finds the ones the plan moved
/// onto the group — the host's database still shows the old numbers
/// while check mode runs.
pub(super) fn primary_group_holder_from(passwd: &str, gid: u64, group: &str) -> Option<String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || super::entries::account_removed(f[0]) {
            continue;
        }
        // The account's primary group under the plan: the plan's own
        // record where a task of the plan gave the account one, and
        // the live entry otherwise.
        let primary = if let Some(planned) = super::plan::primary(f[0]) {
            planned
        } else {
            let Ok(live) = f[3].parse::<u64>() else {
                continue;
            };
            super::plan::PlannedPrimary {
                name: group.to_string(),
                gid: Some(live),
            }
        };
        if primary.holds(gid, group) {
            return Some(f[0].to_string());
        }
    }
    None
}

/// The account that holds `uid` on the host and that the plan has not
/// freed: `useradd -u` and `usermod -u` refuse a number another
/// account holds, so the request is checked against the live database —
/// but a number the plan's own earlier steps free is no collision:
/// a removal really takes the account away, and an earlier renumbering
/// (`usermod -u`) sets the account's new number and frees the old one
/// before the later claim gets its turn.
pub(super) fn uid_holder_in_use(uid: u64, name: &str) -> Result<Option<String>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    Ok(uid_holder_in_use_from(&passwd, uid, name))
}

/// The passwd counterpart of [`uid_holder_in_use`], over canned content.
/// Every account that carries the number is scanned: a host with
/// duplicate numbers has more than one, and the one the plan has not
/// freed is the collision the request runs into.
pub(super) fn uid_holder_in_use_from(passwd: &str, uid: u64, name: &str) -> Option<String> {
    super::entries::uid_holders_from(passwd, uid, name)
        .into_iter()
        .find(|account| !super::entries::account_freed(account, uid))
}

/// The group that holds `gid` on the host and that the plan has not
/// freed: `groupadd -g` and `groupmod -g` refuse a number another
/// group holds, and a group the plan's own earlier steps free — by
/// removal or by renumbering — is no collision.
pub(super) fn gid_holder_in_use(gid: u64, name: &str) -> Result<Option<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(gid_holder_in_use_from(&groups, gid, name))
}

/// The group counterpart of [`uid_holder_in_use`], over canned content.
/// Every group that carries the number is scanned, for the same reason
/// the account scan scans every holder.
pub(super) fn gid_holder_in_use_from(groups: &str, gid: u64, name: &str) -> Option<String> {
    super::entries::gid_holders_from(groups, gid, name)
        .into_iter()
        .find(|group| !super::entries::group_freed(group, gid))
}

/// Whether the plan provides the named group: the host's group database
/// has it, it is a number the host may take as it is, or an earlier task
/// of the plan declares it present — check mode runs no `groupadd`, so
/// the plan's own record is what a later step resolves against. A group
/// no task provides is refused here the way apply refuses it: the
/// identical apply deterministically fails with `group ... not found`,
/// and the dry run must report that instead of a valid planned change.
pub(super) fn group_provided(group: &str, check_mode: bool) -> Result<(), String> {
    if group.parse::<u32>().is_ok() {
        return Ok(());
    }
    if super::group_entry(group)?.is_some() {
        return Ok(());
    }
    if check_mode && super::entries::planned_group(group).is_some() {
        return Ok(());
    }
    Err(format!("group {group:?} not found"))
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

/// Whether the account's claimed private group is missing on the host:
/// a step that names no primary group claims the same-name group — the
/// footprint model reserves it and the lowering's group ordering
/// advertises it as provided — and `useradd -U` reconciles that claim
/// only where the account is created. An existing account reconciles it
/// too, or a dependent `useradd -G alice bob` fails to resolve the group
/// where the account predates the workflow.
pub(super) fn private_group_missing(group: Option<&str>, group_exists: bool) -> bool {
    group.is_none() && !group_exists
}

/// The requested primary group's gid: the live one where the group
/// exists, and `None` in check mode where the host does not have it
/// yet but an earlier task of the plan declares it present — an
/// earlier same-stage `Group` step creates it and check mode runs no
/// `groupadd`, so planning the account change must not require the new
/// group's live GID. In check mode the plan's explicitly requested
/// number also wins over the unchanged live entry: an earlier `Group`
/// step renumbers an existing group (`groupmod -g`) and check mode
/// runs nothing, so the live database still holds the old number while
/// the apply moves the group first — the primary-group change and the
/// home repair must be planned against the number the apply ends up
/// with, not the stale one. Apply mode, which runs the providing step
/// first, resolves from the host and fails where it cannot, before
/// anything mutates. A group no task of the plan provides is refused
/// here the way apply refuses it: the identical apply deterministically
/// fails with `group ... not found`, and the dry run must report that
/// instead of a valid planned change.
pub(super) fn planned_gid(group: &str, check_mode: bool) -> Result<Option<u32>, String> {
    if let Ok(gid) = group.parse::<u32>() {
        return Ok(Some(gid));
    }
    if check_mode
        && let Some(super::entries::Requested::Number(gid)) = super::entries::planned_group(group)
    {
        return Ok(Some(gid));
    }
    match super::group_entry(group)? {
        Some((_, gid)) => u32::try_from(gid)
            .map(Some)
            .map_err(|_| format!("group {group:?} gid {gid} does not fit")),
        None if check_mode && super::entries::planned_group(group).is_some() => Ok(None),
        None => Err(format!("group {group:?} not found")),
    }
}

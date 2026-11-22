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
/// while check mode runs. The same holds for a group the plan
/// renumbered: `groupmod -g` gives every account that holds the group
/// as its primary group the new number, so the account holds the group
/// under the plan even where the host's passwd entry still shows the
/// old one.
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
            // The number the account holds under the plan: the new
            // number of the group the plan renumbered, and the live
            // one where the plan renumbered no group.
            let gid = super::plan::renumbered_gid(live).unwrap_or(live);
            super::plan::PlannedPrimary {
                name: group.to_string(),
                gid: Some(gid),
            }
        };
        if primary.holds(gid, group) {
            return Some(f[0].to_string());
        }
    }
    None
}

/// The passwd counterpart of the primary-holder scan, over canned
/// content: the accounts that hold `gid` as their primary group, apart
/// from `name` itself.
pub(super) fn other_primary_group_holders_from(
    passwd: &str,
    gid: u64,
    group: &str,
    name: &str,
) -> Vec<String> {
    let mut holders = Vec::new();
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || f[0] == name {
            continue;
        }
        // An account the plan has removed holds the group no longer.
        if super::entries::account_removed(f[0]) {
            continue;
        }
        // The account's primary group under the plan: the plan's own
        // record where a task of the plan gave it one, and the live
        // entry otherwise — the number the account holds under the plan
        // is the new number of the group the plan renumbered, and the
        // live one where the plan renumbered no group: the apply moves
        // the group first (`groupmod -g`), so the account holds the new
        // number while the host's passwd entry still shows the old one.
        let primary = if let Some(planned) = super::plan::primary(f[0]) {
            planned
        } else {
            let Ok(live) = f[3].parse::<u64>() else {
                continue;
            };
            let planned_gid = super::plan::renumbered_gid(live).unwrap_or(live);
            super::plan::PlannedPrimary {
                name: group.to_string(),
                gid: Some(planned_gid),
            }
        };
        if primary.holds(gid, group) {
            holders.push(f[0].to_string());
        }
    }
    holders
}

/// Whether the account's private group goes with the account: the
/// group is empty once the account leaves it — no member holds it and
/// no account holds it as its primary group. `userdel` deletes the
/// account's private group only when it is empty; a group another
/// member holds is the host's still, and the agent's own rule says
/// the same about a group a live account holds as its primary group:
/// `groupdel` refuses to delete one without `--force`.
pub(super) fn private_group_emptied(name: &str, gid: Option<u64>) -> Result<bool, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    Ok(private_group_emptied_from(&groups, &passwd, name, gid))
}

/// The canned-content counterpart of [`private_group_emptied`]: the
/// group is empty once the account leaves it — no member holds it
/// under the plan and no account holds it as its primary group under
/// the plan.
pub(super) fn private_group_emptied_from(
    groups: &str,
    passwd: &str,
    name: &str,
    gid: Option<u64>,
) -> bool {
    if !private_group_members_from(groups, passwd, name).is_empty() {
        return false;
    }
    let Some(gid) = gid else {
        // The group is not on the host and the plan knows no number
        // for it: the plan's own group, empty by construction.
        return true;
    };
    other_primary_group_holders_from(passwd, gid, name, name).is_empty()
}

/// The members the group `name` carries under the plan: the live
/// members the plan has not moved away, the live accounts the plan
/// has moved onto the group, and the accounts the plan created with
/// the group in their supplementary set. Check mode runs no `usermod`,
/// so the host's group database still lists the set the plan has
/// already left behind — an account an earlier task of the plan added
/// to the group is a member the live file does not show, and one the
/// plan moved off the group (or took away with the account) is a member
/// the live file still shows. Check mode runs no `useradd` either, so
/// a planned account is absent from the passwd scan the same way: the
/// plan's own user record is consulted too, or a planned member is
/// missed and the group is wrongly reported empty.
pub(super) fn private_group_members_from(groups: &str, passwd: &str, name: &str) -> Vec<String> {
    let mut members = Vec::new();
    for member in super::entries::group_members_from(groups, name) {
        // An account the plan has removed holds no group: check mode
        // runs no `userdel`, so the host's group file still lists it.
        if super::entries::account_removed(&member) {
            continue;
        }
        // The member's supplementary set under the plan: the plan's own
        // record where a task of the plan gave the account one, and the
        // live set otherwise.
        match super::plan::supplementary(&member) {
            Some(set) if !set.iter().any(|group| group == name) => {}
            _ => members.push(member),
        }
    }
    // The live accounts the plan moved onto the group: the host's group
    // file does not list them, so the passwd scan finds them through
    // the plan's own record.
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || f[0] == name {
            continue;
        }
        if super::entries::account_removed(f[0]) {
            continue;
        }
        if members.iter().any(|member| member == f[0]) {
            continue;
        }
        if super::plan::supplementary(f[0]).is_some_and(|set| set.iter().any(|group| group == name))
        {
            members.push(f[0].to_string());
        }
    }
    // The accounts the plan created: check mode runs no `useradd`, so
    // the passwd scan above cannot see them — but the plan's own record
    // holds their supplementary sets, and a member the plan gave the
    // group keeps it although no live entry names it.
    for account in super::plan::users() {
        if account == name || members.iter().any(|member| member == &account) {
            continue;
        }
        // An account the plan has removed holds no group: check mode
        // runs no `userdel`, so the passwd scan above still lists it.
        if super::entries::account_removed(&account) {
            continue;
        }
        if super::plan::supplementary(&account)
            .is_some_and(|set| set.iter().any(|group| group == name))
        {
            members.push(account);
        }
    }
    members
}

/// The accounts that share the same-name group `name` with the account
/// the step manages: its listed members and the accounts that hold it
/// as their primary group, under the plan — apart from the account
/// itself. A shared same-name group is the host's, not the account's
/// to claim implicitly: an implicit `useradd` or `usermod -g` would
/// hand the account a group another principal belongs to, and a home
/// created for it that GID, so the step must declare the group to
/// adopt a shared one.
pub(super) fn same_name_group_sharers(name: &str, gid: u64) -> Result<Vec<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    Ok(same_name_group_sharers_from(&groups, &passwd, name, gid))
}

/// The canned-content counterpart of [`same_name_group_sharers`]: the
/// listed members and primary holders apart from the account itself.
pub(super) fn same_name_group_sharers_from(
    groups: &str,
    passwd: &str,
    name: &str,
    gid: u64,
) -> Vec<String> {
    let mut sharers = private_group_members_from(groups, passwd, name);
    for holder in other_primary_group_holders_from(passwd, gid, name, name) {
        if !sharers.iter().any(|sharer| sharer == &holder) {
            sharers.push(holder);
        }
    }
    sharers
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

/// Whether the host deletes an account's empty same-name group with
/// the account: `USERGROUPS_ENAB` in `/etc/login.defs`. A host that
/// keeps the empty group keeps it, so check mode must predict the group
/// retained — otherwise a later `Group state=absent` is treated as a
/// no-op in check mode while apply re-inspects the retained group and
/// runs `groupdel`. An unreadable policy fails closed: the group is
/// predicted retained.
pub(super) fn usergroups_enabled() -> bool {
    match std::fs::read_to_string("/etc/login.defs") {
        Ok(content) => usergroups_enabled_from(&content),
        Err(_) => false,
    }
}

/// The canned-content counterpart of [`usergroups_enabled`]: the last
/// uncommented `USERGROUPS_ENAB` line wins, `yes` enables and anything
/// else (including absent) disables — fail closed.
pub(super) fn usergroups_enabled_from(content: &str) -> bool {
    let mut enabled = false;
    for line in content.lines() {
        let stripped = line.split('#').next().unwrap_or("").trim();
        if stripped.is_empty() {
            continue;
        }
        let mut parts = stripped.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        if key == "USERGROUPS_ENAB" {
            enabled = value.eq_ignore_ascii_case("yes");
        }
    }
    enabled
}

/// Whether check mode predicts the removal takes the private group: the
/// group is empty once the account leaves it, and the host policy
/// deletes one (`USERGROUPS_ENAB`). The canned policy string drives the
/// tests; an unreadable host policy fails closed through
/// [`usergroups_enabled`].
pub(super) fn check_predicts_private_group_removal(
    emptied: bool,
    usergroups_enabled: bool,
) -> bool {
    emptied && usergroups_enabled
}

/// The group counterpart of [`uid_holder_in_use`], over canned content.
/// Every group that carries the number is scanned, for the same reason
/// the account scan scans every holder.
pub(super) fn gid_holder_in_use_from(groups: &str, gid: u64, name: &str) -> Option<String> {
    super::entries::gid_holders_from(groups, gid, name)
        .into_iter()
        .find(|group| !super::entries::group_freed(group, gid))
}

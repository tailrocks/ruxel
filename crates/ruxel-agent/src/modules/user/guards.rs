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

/// The host's system-account number ranges from `/etc/login.defs`.
/// The guard refuses a live id at or below the ceiling: the
/// statically assigned system ids live below the floor — `root` at 0,
/// `daemon` at 1 — and the allocator's range covers the rest.
pub(super) struct SystemRanges {
    pub uid_min: u64,
    pub uid_max: u64,
    pub gid_min: u64,
    pub gid_max: u64,
}

/// The host's configured system-account ranges, read from
/// `/etc/login.defs`. An unreadable policy — or a present but
/// unparsable key, or a floor above its ceiling — fails closed: the
/// removal is refused instead of treating the number as a removable
/// login account. A key the file omits falls back to the shadow-utils
/// default (see [`system_id_ranges_from`]).
pub(super) fn system_id_ranges() -> Result<SystemRanges, String> {
    // The tests pin the host policy for the thread they run on, so a
    // removal is provable without the host's own file — including on
    // hosts that carry none. Production always reads the host's file.
    #[cfg(test)]
    if let Some(content) = test_policy() {
        return system_id_ranges_from(&content).ok_or_else(|| {
            "cannot read the host's system-account range from /etc/login.defs: \
             each present SYS_UID_MIN, SYS_UID_MAX, SYS_GID_MIN, SYS_GID_MAX, UID_MIN and GID_MIN key must name one number, with each floor at or below its ceiling"
                .to_string()
        });
    }
    let content = std::fs::read_to_string("/etc/login.defs").map_err(|error| {
        format!("cannot read the host's system-account range from /etc/login.defs: {error}")
    })?;
    system_id_ranges_from(&content).ok_or_else(|| {
        "cannot read the host's system-account range from /etc/login.defs: \
         each present SYS_UID_MIN, SYS_UID_MAX, SYS_GID_MIN, SYS_GID_MAX, UID_MIN and GID_MIN key must name one number, with each floor at or below its ceiling"
            .to_string()
    })
}

// The pinned host policy for the test on this thread: the canned
// `/etc/login.defs` content `system_id_ranges` parses instead of the
// host's own file. Thread-local, so parallel tests keep their own
// policy; the pin lasts until the guard drops, so no pin leaks into
// the next test on the thread. (A plain comment: rustdoc documents no
// macro invocation.)
#[cfg(test)]
thread_local! {
    static TEST_POLICY: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn pin_test_policy(content: &str) -> TestPolicyPin {
    TEST_POLICY.with(|slot| *slot.borrow_mut() = Some(content.to_string()));
    TestPolicyPin
}

/// The standard host policy for tests of plan mechanics: the Debian
/// ranges, under which live system accounts are kept and planned
/// login numbers are removable. Boundary tests pin their own policy.
#[cfg(test)]
pub(super) fn pin_debian_policy() -> TestPolicyPin {
    pin_test_policy("SYS_UID_MIN 100\nSYS_UID_MAX 999\nSYS_GID_MIN 100\nSYS_GID_MAX 999\n")
}

#[cfg(test)]
pub(super) struct TestPolicyPin;

#[cfg(test)]
impl Drop for TestPolicyPin {
    fn drop(&mut self) {
        TEST_POLICY.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
fn test_policy() -> Option<String> {
    TEST_POLICY.with(|slot| slot.borrow().clone())
}

/// The canned-content counterpart of [`system_id_ranges`]: the last
/// uncommented line per key wins — the way `USERGROUPS_ENAB` resolves
/// (see [`super::holders::usergroups_enabled_from`]). An omitted key
/// falls back to shadow-utils: 101 per system floor, each ceiling from
/// its regular floor minus one. A present but unparsable key — or a
/// floor above its ceiling — is no range, which fails closed downstream.
pub(super) fn system_id_ranges_from(content: &str) -> Option<SystemRanges> {
    fn value(content: &str, key: &str, default: u64) -> Option<u64> {
        let mut found = None;
        for line in content.lines() {
            let stripped = line.split('#').next().unwrap_or("").trim();
            if stripped.is_empty() {
                continue;
            }
            let mut parts = stripped.split_whitespace();
            if parts.next() == Some(key) {
                found = Some(parts.next()?.parse::<u64>().ok()?);
            }
        }
        Some(found.unwrap_or(default))
    }
    let uid_floor = value(content, "UID_MIN", 1000)?;
    let gid_floor = value(content, "GID_MIN", 1000)?;
    let ranges = SystemRanges {
        uid_min: value(content, "SYS_UID_MIN", 101)?,
        uid_max: value(content, "SYS_UID_MAX", uid_floor.saturating_sub(1))?,
        gid_min: value(content, "SYS_GID_MIN", 101)?,
        gid_max: value(content, "SYS_GID_MAX", gid_floor.saturating_sub(1))?,
    };
    (ranges.uid_min <= ranges.uid_max && ranges.gid_min <= ranges.gid_max).then_some(ranges)
}

/// Whether the agent may remove the account whose host entry is `entry`.
/// The deletion verifies the host entry rather than trusting the workflow
/// value: the superuser's account is the host's to keep whatever the
/// workflow states, as is an account the plan declared a system account
/// — the live entry carries no system bit, so the plan's own record is
/// what preserves a `--system` creation past the allocator's choice of
/// uid — and a live uid at or below the host's configured system range
/// (see [`system_id_ranges`]): the fixed 1000 boundary mistakes a
/// system account for a login one on hosts whose range extends above
/// UID 999. The declaration is verified before the policy is read: a
/// declared uid must match the host's — `user = "daemon"; uid = 1000`
/// must not remove an account whose real UID is a system one — and a
/// declaration that names another number is refused whatever it is.
pub(super) fn removal_allowed(
    name: &str,
    declared: Option<u64>,
    entry: Option<&PasswdEntry>,
    is_system: bool,
) -> Result<(), String> {
    // A declared system account is refused on the declaration alone:
    // the allocator draws its number at apply, so check mode has no
    // number to verify — and the live entry carries no system bit to
    // probe afterward.
    if is_system {
        return Err(format!(
            "user: refusing to remove {name}: the account is a declared system account, which the host keeps"
        ));
    }
    let Some(entry) = entry else {
        return Ok(());
    };
    if entry.uid == 0 {
        return Err(format!(
            "user: refusing to remove {name}: the superuser's account (uid 0) is the host's to keep"
        ));
    }
    // The declaration is verified before the host policy is read: a
    // declaration that names another number is refused whatever the
    // number is — and without the host's number range the guard cannot
    // run at all.
    if let Some(want) = declared
        && want != entry.uid
    {
        return Err(format!(
            "user: refusing to remove {name}: the host uid {} is not the declared {want}",
            entry.uid
        ));
    }
    let ranges =
        system_id_ranges().map_err(|error| format!("user: refusing to remove {name}: {error}"))?;
    if system_uid_kept(entry.uid, &ranges) {
        return Err(format!(
            "user: refusing to remove {name}: the host account is a system account (uid {}), which the host keeps",
            entry.uid
        ));
    }
    Ok(())
}

/// The number the host policy keeps: the canned-policy counterpart of
/// the range check in [`removal_allowed`], over an explicit host
/// policy, so the boundary is provable without the host's own
/// `/etc/login.defs`.
pub(super) fn system_uid_kept(uid: u64, ranges: &SystemRanges) -> bool {
    uid <= ranges.uid_max
}

/// Whether the agent may remove the group of host gid `host_gid`. The
/// declaration is verified before anything else — a declared gid must
/// match the host's, because the host entry, not the workflow value,
/// is what deletion acts on — then the primary holder: a group a live
/// account holds as its primary group is the account's to keep, since
/// `groupdel` refuses to delete one without `--force`, which this
/// removal does not pass. Only then the host policy: a live gid at or
/// below the configured system range is the host's too (see
/// [`system_id_ranges`]). The root group (GID 0) is the host's to keep
/// whatever it is named. Each refusal fires before the utility runs —
/// in check mode as much as in apply.
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
    let ranges =
        system_id_ranges().map_err(|error| format!("group: refusing to remove {name}: {error}"))?;
    if system_gid_kept(host_gid, &ranges) {
        return Err(format!(
            "group: refusing to remove {name}: the host group is a system group (gid {host_gid}), which the host keeps"
        ));
    }
    Ok(())
}

/// The number the host policy keeps: the gid counterpart of
/// [`system_uid_kept`], so the boundary is provable without the host's
/// own `/etc/login.defs`.
pub(super) fn system_gid_kept(gid: u64, ranges: &SystemRanges) -> bool {
    gid <= ranges.gid_max
}

pub(super) use super::holders::{
    gid_holder_in_use, primary_group_holder, private_group_emptied, uid_holder_in_use,
};
#[cfg(test)]
pub(super) use super::holders::{
    gid_holder_in_use_from, other_primary_group_holders_from, primary_group_holder_from,
    private_group_emptied_from, uid_holder_in_use_from,
};

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

/// The emptiness scan's plan: the holder a renumbering moves and the
/// member the plan adds or takes away, over canned content — the host's
/// databases still show the numbers and the sets the plan has already
/// left behind while check mode runs.
#[cfg(test)]
#[path = "guards/tests/mod.rs"]
mod tests;

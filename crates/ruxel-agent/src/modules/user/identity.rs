//! The account's identity under the plan: the numbers and the group
//! sets the plan ends up with, where an earlier task of the plan
//! changed what the host's database still shows.

use super::entries::planned_uid;
use super::guards::planned_gid;
use super::plan::{self, PlannedPrimary};

/// The account's effective identity under the plan: the number and the
/// primary-group number the plan ends up with.
pub(super) struct Effective {
    pub(super) uid: u64,
    pub(super) gid: u64,
}

/// The account's effective identity: the plan's own record where an
/// earlier task of the plan changed it — check mode runs no `usermod`,
/// so the host's entry still holds the old numbers — and the live
/// entry otherwise. Every guard and every repair resolves against the
/// identity the plan ends up with, not against the host's unchanged
/// one.
pub(super) fn effective(name: &str, live: &super::PasswdEntry) -> Effective {
    Effective {
        uid: planned_uid(name).unwrap_or(live.uid),
        gid: plan::primary(name)
            .and_then(|primary| primary.gid)
            .unwrap_or(live.gid),
    }
}

/// The plan's record of the primary group an account step claims: the
/// declared group, or the account's own same-name group where the step
/// claims none — the number the plan knows for it, and `None` where
/// the group is not on the host yet.
pub(super) fn planned_primary_of(
    name: &str,
    group: Option<&str>,
    check_mode: bool,
) -> PlannedPrimary {
    let claimed = group.unwrap_or(name);
    PlannedPrimary {
        name: claimed.to_string(),
        gid: planned_gid(claimed, check_mode)
            .ok()
            .flatten()
            .map(u64::from),
    }
}

/// The group's effective number under the plan: the number an earlier
/// task of the plan gave it — check mode runs no `groupmod`, so the
/// host's database still holds the number the plan has left behind —
/// and the live one otherwise. The account's effective identity is
/// resolved the same way: against the number the plan ends up with,
/// not against the host's unchanged one.
pub(super) fn effective_group_gid(name: &str, live_gid: u64, check_mode: bool) -> u64 {
    if !check_mode {
        return live_gid;
    }
    match super::entries::planned_group(name) {
        Some(super::entries::Requested::Number(number)) => u64::from(number),
        _ => live_gid,
    }
}

/// The account's supplementary set under the plan: the declared one
/// where the step sets it, and the union of the set the account holds
/// and the declared one where the step appends.
pub(super) fn planned_set(current: Vec<String>, declared: &[String], append: bool) -> Vec<String> {
    if !append {
        return declared.to_vec();
    }
    let mut set = current;
    for group in declared {
        if !set.contains(group) {
            set.push(group.clone());
        }
    }
    set
}

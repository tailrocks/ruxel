//! Delegation destinations named literally in the plan: the sessions
//! an incomplete origin may have reached, even where the origin never
//! evaluated the destination — a tag or condition skipped the task
//! before any request was sent. Expression destinations are recorded
//! where they evaluate (see `Runner::delegated`); literals are read
//! here, play-wide, so a filtered or aborted origin still marks them.

use super::{BTreeMap, BTreeSet, Recap, Value, array};

/// Every host a task of the play names as a literal `delegateTo`: the
/// sessions the play may delegate to, whatever skips the origin before
/// the destination evaluates. Expression destinations never spell a
/// host, so they stay out — the runner records those where they send.
pub(super) fn literal_targets(play: &Value) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for section in ["preTasks", "tasks", "handlers"] {
        for task in array(play, section) {
            collect(task, &mut targets);
        }
    }
    targets
}

fn collect(task: &Value, targets: &mut BTreeSet<String>) {
    if let Some(host) = task.get("delegateTo").and_then(Value::as_str) {
        targets.insert(host.to_owned());
    }
    for section in ["block", "rescue", "always"] {
        for child in array(task, section) {
            collect(child, targets);
        }
    }
}

/// Mark every session the incomplete origin may have delegated to:
/// the play's literal targets — a skipped task never evaluates its
/// destination — and the destinations the run actually sent to. Only
/// sessions with a recap of their own are marked: a delegated host
/// with no recap at all already shuts down incomplete (see
/// `session_incomplete`), so no entry is ever created here and the
/// reported recaps never gain a host that ran nothing. The mark is
/// `filtered`, the same no-prune verdict the origin carries: it only
/// delays expiry to a later complete run, never wrongly authorizes one.
pub(crate) fn mark_delegation_destinations(
    totals: &mut BTreeMap<String, Recap>,
    play: &Value,
    origin: &str,
) {
    let mut destinations = literal_targets(play);
    if let Some(total) = totals.get(origin) {
        destinations.extend(total.delegated.iter().cloned());
    }
    destinations.remove(origin);
    for destination in destinations {
        if let Some(total) = totals.get_mut(&destination) {
            total.filtered = true;
        }
    }
}

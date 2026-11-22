//! The user and group operations' tests: the parsers over canned passwd
//! and group content, set-based supplementary-group reconciliation, and
//! the host-entry verification a removal carries.

use super::*;

mod changes;
mod collisions;
mod creation;
mod frames;
mod group_plan;
mod guards;
mod holders;
mod home_modes;
mod home_repair;
mod home_repair_incidental;
mod home_repair_takeback;
mod homes;
mod identity;
mod parsing;
mod plan_reset;
mod plan_state;
mod plan_state_later;
mod planned;
mod planned_check_mode;
mod reconciliation;
mod recreated;
mod removal;
mod shell_planned;
mod shells;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

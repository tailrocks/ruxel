//! The user and group operations' tests: the parsers over canned passwd
//! and group content, set-based supplementary-group reconciliation, and
//! the host-entry verification a removal carries.

use super::*;

mod changes;
mod collisions;
mod creation;
mod guards;
mod holders;
mod homes;
mod identity;
mod parsing;
mod planned;
mod reconciliation;
mod removal;
mod shells;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

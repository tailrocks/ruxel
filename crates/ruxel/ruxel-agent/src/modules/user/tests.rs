//! The user and group operations' tests: the parsers over canned passwd
//! and group content, set-based supplementary-group reconciliation, and
//! the host-entry verification a removal carries.

use super::*;

mod changes;
mod parsing;
mod reconciliation;
mod removal;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

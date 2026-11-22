//! The plan's record of the accounts it declared system accounts
//! (`useradd --system`): the host keeps those whatever uid the
//! allocator drew for them, so a later removal is refused even above
//! the uid-1000 fallback — the live passwd entry carries no system bit
//! to probe.

use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    /// The accounts the plan declared system accounts: the host keeps
    /// them whatever number they hold, so the guard refuses their
    /// removal even where the allocator drew a regular-looking uid.
    static SYSTEM_USERS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Clears the record with the rest of the plan's state: a new plan
/// starts from no system flag again.
pub(super) fn reset() {
    SYSTEM_USERS.with(|users| users.borrow_mut().clear());
}

/// The plan declared the account a system account: the host keeps it
/// whatever number it holds, so the guard refuses its removal even
/// where the allocator drew a regular-looking uid for it.
pub(super) fn note_system(name: &str) {
    SYSTEM_USERS.with(|users| {
        users.borrow_mut().insert(name.to_string());
    });
}

/// Whether the plan declared the account a system account.
pub(super) fn is_system(name: &str) -> bool {
    SYSTEM_USERS.with(|users| users.borrow().contains(name))
}

/// The record's counterpart when the plan removes the account: the
/// deletion takes the account away, so a later declaration starts
/// from no system flag again.
pub(super) fn forget_system(name: &str) {
    SYSTEM_USERS.with(|users| {
        users.borrow_mut().remove(name);
    });
}

//! The take-back of a home a failed creation made.

use super::super::*;

#[test]
fn a_group_writable_symlink_target_ancestor_is_refused() {
    // The strict base rule the lexical ancestors, the resolved target
    // and the chain's first disjunct enforce: no group write, no other
    // write, operator-owned. The chain's narrow privileged-group
    // exception lives in `chain_dir_trusted` and is pinned with the
    // chain tests in `shells.rs`.
    let operator = adoption::operating_uid();
    for uid in [0, operator] {
        assert!(adoption::own_directory(uid, 0o755));
        assert!(
            !adoption::own_directory(uid, 0o775),
            "a group-writable directory fails the strict rule"
        );
        assert!(!adoption::own_directory(uid, 0o777));
        assert!(!adoption::own_directory(uid, 0o707));
    }
}

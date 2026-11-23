//! Removals of declared system accounts: the plan records the system
//! flag because the live entry carries no system bit, and the guard
//! refuses on that record with the uid-1000 rule as the fallback.

use super::super::*;

#[test]
fn a_declared_system_account_is_never_removed_whatever_its_uid() {
    // Thread 326: a system account is not necessarily below UID 1000 —
    // the allocator draws whatever number is free, and `login.defs`
    // moves the ranges — so the plan records the declared system flag
    // and the guard refuses the removal on that record, with the
    // uid-1000 rule kept only as the fallback for accounts the
    // workflow never declared.
    begin_plan();
    let name = format!("ruxel-system-{}", std::process::id());
    assert!(
        passwd_entry(&name).unwrap().is_none(),
        "the account must not exist on the host"
    );
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    run(
        &json!({"name": name, "system": true, "create_home": false}),
        &ctx,
    )
    .expect("the system creation must plan");
    assert!(plan_system::is_system(&name));
    let error = run(&json!({"name": name, "state": "absent"}), &ctx)
        .expect_err("a declared system account must not be removed");
    assert!(error.contains("refusing to remove"), "{error}");
    assert!(error.contains("system account"), "{error}");
    begin_plan();
}

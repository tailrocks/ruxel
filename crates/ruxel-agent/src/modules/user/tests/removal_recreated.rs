//! Removals after a remove-and-recreate sequence: the allocator draws
//! the number anew, so check mode cannot verify against a live entry
//! while apply must verify against the one the recreation drew.

use super::super::*;

fn apply_context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

#[test]
fn apply_verifies_an_unnumbered_recreation_against_the_live_uid() {
    // Thread 322: after a remove → unnumbered recreate sequence in
    // apply mode, `user_recreated` stays true although the live entry
    // holds the allocator-chosen uid of the recreated account. The
    // unknown-uid escape must only apply in check mode: in apply the
    // live uid is what `userdel` verifies the declaration against, the
    // way the group arm already verifies a recreated group.
    begin_plan();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let (name, live, gid) = passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid > 0)).then(|| {
                (
                    f[0].to_string(),
                    f[2].parse::<u64>().unwrap(),
                    f[3].parse::<u64>().unwrap(),
                )
            })
        })
        .expect("a non-root account on the host");
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let primary = super::super::guards::group_name_of_gid_from(&groups, gid);
    // The plan removed the account and declares it present again
    // without a number: the apply recreated it, so the live database
    // holds the allocator's choice for this name.
    plan::note_removed_account(&name);
    plan::note_user(&name, None);
    assert!(plan::user_recreated(&name));
    super::super::test_hook::arm(vec![Ok(())]);
    // An incorrect uid: the declaration the apply verifies against the
    // live entry must be refused before `userdel` runs.
    let error = run(
        &json!({"name": name, "state": "absent", "uid": live + 1, "group": primary}),
        &apply_context(),
    )
    .expect_err("apply must verify the declaration against the live entry");
    assert!(error.contains("refusing to remove"), "{error}");
    assert!(
        super::super::test_hook::take_calls().is_empty(),
        "userdel must not run after a refused verification"
    );
    begin_plan();
}

#[test]
fn check_mode_still_skips_the_uid_guard_for_an_unnumbered_recreation() {
    // The other direction of thread 322: in check mode nothing runs,
    // so the allocator has drawn nothing yet and the stale live entry
    // must not stand in for the number the apply will draw — the
    // removal of the plan's own recreated account plans without a
    // verification no number can satisfy.
    begin_plan();
    let name = format!("ruxel-recreated-{}", std::process::id());
    assert!(
        passwd_entry(&name).unwrap().is_none(),
        "the account must not exist on the host"
    );
    plan::note_removed_account(&name);
    plan::note_user(&name, None);
    assert!(plan::user_recreated(&name));
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let removal = run(&json!({"name": name, "state": "absent"}), &ctx)
        .expect("check mode must plan the recreated account's removal");
    assert_eq!(removal.get("changed"), Some(&json!(true)), "{removal}");
    begin_plan();
}

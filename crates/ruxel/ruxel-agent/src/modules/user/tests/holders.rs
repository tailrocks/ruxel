//! The holder scans: the accounts and the groups that hold a number
//! the plan claims, and the groups an account holds as its primary
//! group.

use super::super::*;

fn check_context() -> ExecContext {
    ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

#[test]
fn a_group_the_plan_moves_an_account_off_of_is_removed() {
    // An earlier check-mode task moves an account off the group, and a
    // later task removes the group: the apply moves the account first
    // (`usermod -g`) and its `groupdel` then succeeds, so the dry run
    // must not refuse the removal — the holder scan reads the plan's
    // own record, not the host's unchanged passwd file.
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (group_name, _gid, account) = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let name = f.first()?.to_string();
            let gid = f.get(2)?.parse::<u64>().ok()?;
            if gid == 0 || name.starts_with(['+', '#', '-']) {
                return None;
            }
            let holders: Vec<&str> = passwd
                .lines()
                .filter(|other| {
                    let g: Vec<&str> = other.split(':').collect();
                    g.len() >= 4 && g[3].parse::<u64>() == Ok(gid)
                })
                .collect();
            (holders.len() == 1).then(|| {
                let holder: Vec<&str> = holders[0].split(':').collect();
                (name, gid, holder[0].to_string())
            })
        })
        .expect("a group exactly one account holds as its primary group");
    let other = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let name = f.first()?.to_string();
            let number = f.get(2)?.parse::<u64>().ok()?;
            (number != 0 && name != group_name && !name.starts_with(['+', '#', '-']))
                .then_some(name)
        })
        .expect("another group on the host");
    let ctx = check_context();
    // The earlier task of the plan: the account's primary group moves.
    run(&json!({"name": account, "group": other}), &ctx).expect("the primary-group move must plan");
    // The later task: the group the account has left is removed.
    let removed = group(&json!({"name": group_name, "state": "absent"}), &ctx)
        .expect("the group the plan moved the account off of must be removed");
    assert_eq!(removed.get("changed"), Some(&json!(true)), "{removed}");
}

#[test]
fn a_duplicate_number_the_plan_frees_is_still_a_collision() {
    // A host with duplicate numbers: the first account carries the
    // number and the plan takes it away, and a second account still
    // carries it — `useradd -u` and `usermod -u` refuse the number
    // (they need `--non-unique`), so the scan must look past the
    // holder the plan freed and find the one that remains.
    let passwd = "alice:x:1001:1001::/home/alice:/bin/sh\nbob:x:1001:1001::/home/bob:/bin/sh\n";
    assert_eq!(
        super::super::guards::uid_holder_in_use_from(passwd, 1001, "carol"),
        Some("alice".to_string())
    );
    super::super::entries::note_removed_account("alice");
    assert_eq!(
        super::super::guards::uid_holder_in_use_from(passwd, 1001, "carol"),
        Some("bob".to_string()),
        "the scan continues to the holder the plan has not freed"
    );
    // The group counterpart: `groupadd -g` refuses a number another
    // group holds the same way.
    let groups = "team:x:1001:\ncrew:x:1001:\n";
    assert_eq!(
        super::super::guards::gid_holder_in_use_from(groups, 1001, "other"),
        Some("team".to_string())
    );
    super::super::entries::note_removed_group("team");
    assert_eq!(
        super::super::guards::gid_holder_in_use_from(groups, 1001, "other"),
        Some("crew".to_string()),
        "the scan continues to the holder the plan has not freed"
    );
}

#[test]
fn a_plan_created_member_keeps_the_private_group() {
    // Check mode runs no `useradd`, so an account an earlier task of
    // the plan created is absent from the passwd scan — but the plan's
    // own record holds its supplementary set. The plan creates `alice`,
    // creates `bob` with supplementary group `alice`, and removes
    // `alice`: the dry run must see that `bob` still holds the group —
    // `userdel` keeps a private group another member holds — instead of
    // predicting that the removal takes the group with the account.
    let groups = "alice:x:1001:\n";
    let passwd = "alice:x:1001:1001::/home/alice:/bin/sh\n";
    super::super::plan::note_user("alice", Some(1001));
    super::super::plan::note_user("bob", Some(1002));
    super::super::plan::note_supplementary("bob", vec!["alice".to_string()]);
    super::super::plan::note_removed_account("alice");
    let members = super::super::holders::private_group_members_from(groups, passwd, "alice");
    assert_eq!(
        members,
        vec!["bob".to_string()],
        "the plan's own member keeps the group: {members:?}"
    );
    assert!(
        !super::super::holders::private_group_emptied_from(groups, passwd, "alice", Some(1001)),
        "the group survives the removal while the plan's member holds it"
    );
}

#[test]
fn usergroups_enab_decides_the_check_mode_prediction() {
    // `USERGROUPS_ENAB=yes` hosts delete the empty same-name group with
    // the account; `no` hosts retain it. The canned policy drives the
    // check-mode prediction for both, and a missing or commented line
    // fails closed — the group is predicted retained.
    assert!(
        super::super::holders::usergroups_enabled_from("USERGROUPS_ENAB yes\n"),
        "yes enables the private-group deletion"
    );
    assert!(
        super::super::holders::usergroups_enabled_from("USERGROUPS_ENAB YES\n"),
        "the value compares case-insensitively"
    );
    assert!(
        !super::super::holders::usergroups_enabled_from("USERGROUPS_ENAB no\n"),
        "no retains the empty group"
    );
    assert!(
        !super::super::holders::usergroups_enabled_from("SYS_GID_MAX 999\n"),
        "an absent line fails closed"
    );
    assert!(
        !super::super::holders::usergroups_enabled_from("#USERGROUPS_ENAB yes\n"),
        "a commented line is not policy"
    );
    assert!(
        super::super::holders::usergroups_enabled_from("USERGROUPS_ENAB yes # trailing\n"),
        "a trailing comment does not disable the line"
    );
    assert!(
        super::super::holders::usergroups_enabled_from("USERGROUPS_ENAB no\nUSERGROUPS_ENAB yes\n"),
        "the last uncommented line wins"
    );
    assert!(
        !super::super::holders::usergroups_enabled_from(
            "USERGROUPS_ENAB yes\nUSERGROUPS_ENAB no\n"
        ),
        "the last uncommented line wins either way"
    );
}

#[test]
fn a_shared_same_name_group_names_every_other_holder() {
    // Thread 288: implicit adoption must see every principal that
    // holds the same-name group — a listed supplementary member and
    // an account that holds it as its primary group — while the
    // account itself is no sharer of its own group.
    let groups = "app:x:2001:bob\nops:x:2002:\n";
    let passwd = "app:x:2001:2001::/home/app:/bin/sh\nbob:x:2002:2001::/home/bob:/bin/sh\ncarol:x:2003:2003::/home/carol:/bin/sh\n";
    let sharers = super::super::holders::same_name_group_sharers_from(groups, passwd, "app", 2001);
    assert_eq!(
        sharers,
        vec!["bob".to_string()],
        "the listed member and the primary holder share the group: {sharers:?}"
    );
    let exclusive =
        super::super::holders::same_name_group_sharers_from(groups, passwd, "ops", 2002);
    assert!(
        exclusive.iter().all(|sharer| sharer != "ops"),
        "no account shares the exclusive group: {exclusive:?}"
    );
    assert!(
        !exclusive.contains(&"carol".to_string()),
        "an account on another group is no sharer: {exclusive:?}"
    );
}

#[test]
fn a_plan_created_primary_holder_shares_the_same_name_group() {
    // Thread 288: check mode runs no `useradd`, so an account the plan
    // creates is absent from the passwd scan — but the plan's own
    // record holds its primary group, and a planned holder of the
    // same-name group keeps it the way a live one does. The dry run
    // must see that `dave`, created with primary group `app`, shares
    // the group instead of approving an implicit adoption of it.
    begin_plan();
    super::super::plan::note_user("ruxel-sharer-dave", Some(3001));
    super::super::plan::note_primary(
        "ruxel-sharer-dave",
        super::super::plan::PlannedPrimary {
            name: "app".to_string(),
            gid: Some(2001),
        },
    );
    let groups = "app:x:2001:bob\n";
    let passwd = "app:x:2001:2001::/home/app:/bin/sh\nbob:x:2002:2001::/home/bob:/bin/sh\n";
    let sharers = super::super::holders::same_name_group_sharers_from(groups, passwd, "app", 2001);
    assert!(
        sharers.contains(&"ruxel-sharer-dave".to_string()),
        "the plan-created primary holder shares the group: {sharers:?}"
    );
    assert!(
        sharers.contains(&"bob".to_string()),
        "the live primary holder still shares the group: {sharers:?}"
    );
    assert!(
        !sharers.iter().any(|sharer| sharer == "app"),
        "the account itself is no sharer of its own group: {sharers:?}"
    );
    // A planned account the plan removed holds the group no longer.
    super::super::plan::note_removed_account("ruxel-sharer-dave");
    let sharers = super::super::holders::same_name_group_sharers_from(groups, passwd, "app", 2001);
    assert!(
        !sharers.contains(&"ruxel-sharer-dave".to_string()),
        "a removed planned holder shares nothing: {sharers:?}"
    );
    begin_plan();
}

#[test]
fn the_check_mode_prediction_consults_the_host_policy() {
    // An empty same-name group disappears with the account only where
    // the host policy deletes one: `USERGROUPS_ENAB=no` (or an
    // unreadable policy) predicts the group retained, so a later
    // `Group state=absent` still plans the removal the apply runs.
    assert!(
        super::super::holders::check_predicts_private_group_removal(true, true),
        "an empty group on an enabled host is predicted gone"
    );
    assert!(
        !super::super::holders::check_predicts_private_group_removal(true, false),
        "an empty group on a disabled host is predicted retained"
    );
    assert!(
        !super::super::holders::check_predicts_private_group_removal(false, true),
        "a held group survives whatever the policy says"
    );
    assert!(
        !super::super::holders::usergroups_enabled_from(""),
        "an unreadable policy fails closed through the empty content"
    );
}

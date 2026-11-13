//! The collisions the plan's own earlier steps resolve: a group a live
//! account holds as its primary group, a group the host does not have,
//! and a number an earlier removal of the plan frees.

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
fn a_group_a_live_account_holds_as_its_primary_group_is_never_removed() {
    // `groupdel` refuses to delete a group that is an account's primary
    // group without `--force`, which this removal does not pass, so the
    // refusal must come before the utility runs — in check mode as
    // much as in apply.
    let passwd = "alice:x:1001:1002::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1002),
        Some("alice".to_string())
    );
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1003),
        None
    );
    assert!(group_removal_allowed("staff", None, 1002, None).is_ok());
    let error = group_removal_allowed("staff", None, 1002, Some("alice"))
        .expect_err("a group that is a live primary group must not be removed");
    assert!(error.contains("primary group"), "{error}");
}

#[test]
fn a_live_primary_group_is_refused_before_groupdel_runs() {
    // The host's own account whose primary group is a live group: the
    // check-mode plan reports a pending removal of that group although
    // the identical apply fails at `groupdel`.
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let holder = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 4 && f[3].parse::<u64>().is_ok_and(|gid| gid != 0))
                .then(|| f[0].to_string())
        })
        .find(|account| {
            super::super::guards::primary_group_holder_from(&passwd, gid_of(&passwd, account))
                .is_some()
        })
        .expect("a live account with a primary group");
    let gid = gid_of(&passwd, &holder);
    let name = super::super::guards::group_name_of_gid_from(&groups, gid)
        .expect("the live group of the account's primary group");
    let error = group(&json!({"name": name, "state": "absent"}), &check_context())
        .expect_err("a group that is a live account's primary group must not be removed");
    assert!(error.contains("primary group"), "{error}");
}

/// The primary group number the account's passwd entry carries.
fn gid_of(passwd: &str, account: &str) -> u64 {
    passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 4 && f[0] == account).then(|| f[3].parse::<u64>().expect("a group number"))
        })
        .expect("the account's passwd entry")
}

#[test]
fn a_created_account_refuses_a_group_the_host_does_not_have() {
    // `useradd` resolves the `-g` and `-G` names against the host's
    // group database and fails on an unresolved one, so the dry run
    // must refuse the same way instead of reporting a creation that
    // cannot succeed.
    let group = format!("ruxel-absent-group-{}", std::process::id());
    assert!(
        group_entry(&group).unwrap().is_none(),
        "the group must not exist on the host"
    );
    let ctx = check_context();
    let error = run(
        &json!({"name": "ruxel-absent-account", "group": group}),
        &ctx,
    )
    .expect_err("a creation whose primary group does not exist must be refused");
    assert!(error.contains("not found"), "{error}");
    let error = run(
        &json!({"name": "ruxel-absent-account", "groups": [group]}),
        &ctx,
    )
    .expect_err("a creation whose supplementary group does not exist must be refused");
    assert!(error.contains("not found"), "{error}");
}

#[test]
fn a_changed_account_refuses_a_group_the_host_does_not_have() {
    // `usermod -G` resolves the list against the host and fails on an
    // unresolved name, so the dry run of a change that names one must
    // refuse the same way instead of reporting a valid planned change.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| f[0].to_string())
        })
        .expect("a non-root account on the host");
    let group = format!("ruxel-absent-group-{}", std::process::id());
    assert!(
        group_entry(&group).unwrap().is_none(),
        "the group must not exist on the host"
    );
    let error = run(&json!({"name": name, "groups": [group]}), &check_context())
        .expect_err("a change whose supplementary group does not exist must be refused");
    assert!(error.contains("not found"), "{error}");
}

#[test]
fn a_number_the_plan_freed_is_no_collision() {
    // The plan removes a group and then claims its number for another
    // group: the removal really takes the group away, so the number is
    // free — check mode runs no `groupdel`, so the plan's own record is
    // what the later claim resolves against.
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (name, gid) = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let name = f.first()?.to_string();
            let gid = f.get(2)?.parse::<u64>().ok()?;
            (gid != 0
                && !name.starts_with(['+', '#', '-'])
                && super::super::guards::primary_group_holder_from(&passwd, gid).is_none())
            .then_some((name, gid))
        })
        .expect("a live group no account holds as its primary group");
    let ctx = check_context();
    let removed = group(&json!({"name": name, "state": "absent"}), &ctx)
        .expect("the removal of the group must plan");
    assert_eq!(removed.get("changed"), Some(&json!(true)), "{removed}");
    let created = group(
        &json!({"name": format!("ruxel-freed-group-{}", std::process::id()), "gid": gid}),
        &ctx,
    )
    .expect("the number the plan freed must be available for the later group");
    assert_eq!(created.get("changed"), Some(&json!(true)), "{created}");
}

#[test]
fn an_account_the_plan_removed_holds_no_number() {
    // The account the plan removes in an earlier stage frees its
    // number for a later stage: the collision check reads the host's
    // database, which still lists the account while check mode runs.
    let passwd = "ruxel-freed-account:x:4242:4242::/home/ruxel-freed-account:/bin/sh\n";
    assert_eq!(
        super::super::guards::uid_holder_in_use_from(passwd, 4242, "bob"),
        Some("ruxel-freed-account".to_string())
    );
    super::super::guards::note_removed_account("ruxel-freed-account");
    assert_eq!(
        super::super::guards::uid_holder_in_use_from(passwd, 4242, "bob"),
        None,
        "a number the plan freed is no collision"
    );
}

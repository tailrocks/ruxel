//! The account's identity under the plan: the numbers and the group
//! sets the plan ends up with, where an earlier task of the plan
//! changed what the host's database still shows.

use super::super::*;

/// The account the plan moves: an existing account whose home is a
/// directory of its own, so the home's repair is the only change a
/// step that declares nothing else plans.
fn account_with_an_owned_home() -> Option<(String, u64, u64, String, String)> {
    use std::os::unix::fs::MetadataExt;
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 {
            continue;
        }
        let (Ok(uid), Ok(gid)) = (f[2].parse::<u64>(), f[3].parse::<u64>()) else {
            continue;
        };
        if uid == 0 {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(f[5]) else {
            continue;
        };
        if !meta.is_dir() || u64::from(meta.uid()) != uid || u64::from(meta.gid()) != gid {
            continue;
        }
        let group = super::super::guards::group_name_of_gid_from(&groups, gid)?;
        return Some((f[0].to_string(), uid, gid, f[5].to_string(), group));
    }
    None
}

#[test]
fn the_home_repair_resolves_against_the_planned_uid() {
    // The number the plan gave the account: an earlier task renumbered
    // it, and the live entry still holds the old number while check
    // mode runs — the repair resolves against the number the plan ends
    // up with.
    note_planned_user("ruxel-planned-uid-account", Some(2000));
    let live = super::super::PasswdEntry {
        uid: 1001,
        gid: 1001,
        ..Default::default()
    };
    assert_eq!(
        super::super::identity::effective("ruxel-planned-uid-account", &live).uid,
        2000,
        "the planned number wins over the stale live one"
    );
    assert_eq!(
        super::super::identity::effective("ruxel-unplanned-uid-account", &live).uid,
        1001,
        "the live number stands where the plan states none"
    );
    forget_planned_user("ruxel-planned-uid-account");
}

#[test]
fn check_mode_plans_the_home_repair_against_the_planned_uid() {
    // An earlier check-mode task renumbers an existing account, and a
    // later task declares the account's home without stating a
    // number: check mode runs no `usermod`, so the host's entry still
    // holds the old number — the home's repair must be planned
    // against the number the apply ends up with, or the dry run
    // reports no change while the apply chowns the home to the new
    // number.
    let Some((name, uid, _, home, group)) = account_with_an_owned_home() else {
        // A host with no account whose home it owns: the repair's
        // ownership cannot be observed here.
        return;
    };
    let moved = uid + 100_000;
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    assert!(
        !passwd
            .lines()
            .any(|line| line.split(':').nth(2) == Some(moved.to_string().as_str())),
        "the planned number must be free on the host"
    );
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The earlier task of the plan: the account's number changes.
    run(&json!({"name": name, "uid": moved}), &check).expect("the earlier renumbering must plan");
    // The later task: the home is declared, the number is not.
    let result = run(&json!({"name": name, "home": home, "group": group}), &check)
        .expect("the later step must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the home's repair must be planned against the planned number: {result}"
    );
}

#[test]
fn check_mode_plans_the_supplementary_set_against_the_planned_one() {
    // An earlier check-mode task adds the account to a group, and a
    // later task declares an empty group set: check mode runs no
    // `usermod`, so the host's group database still lists the account
    // without the group — the later task must be compared against the
    // set the plan has already given the account, or the dry run
    // reports no change while the apply removes the membership.
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let account = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 || f[2].parse::<u64>().is_err() {
                return None;
            }
            // The account holds no supplementary group on the host:
            // the later task's empty set is a change of the plan's own
            // set, not of the host's.
            if supplementary_groups_from(&groups, f[0]).is_empty() {
                Some((f[0].to_string(), f[3].parse::<u64>().unwrap()))
            } else {
                None
            }
        })
        .find(|(_, gid)| super::super::guards::group_name_of_gid_from(&groups, *gid).is_some())
        .expect("an account without supplementary groups on the host");
    let (name, gid) = account;
    let primary = super::super::guards::group_name_of_gid_from(&groups, gid)
        .expect("the account's primary group");
    let other = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let group = f.first()?;
            let number = f.get(2)?.parse::<u64>().ok()?;
            (group != &primary
                && number != 0
                && !group.starts_with(['+', '#', '-'])
                && !f.get(3)?.split(',').any(|member| member == name))
            .then(|| group.to_string())
        })
        .expect("a group the account is not a member of");
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The earlier task of the plan: the account joins the group.
    run(
        &json!({"name": name, "group": primary, "groups": [other]}),
        &check,
    )
    .expect("the earlier membership must plan");
    // The later task: the group set is emptied.
    let result = run(
        &json!({"name": name, "group": primary, "groups": []}),
        &check,
    )
    .expect("the later step must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the membership the plan added must be planned away: {result}"
    );
}

#[test]
fn an_account_the_plan_moves_off_a_group_holds_it_no_longer() {
    // The scan of the accounts that hold a group as their primary
    // group reads the plan's own record: an account the plan gives
    // another primary group holds this one no longer, although the
    // host's database still lists it while check mode runs.
    let passwd = "alice:x:1001:1002::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1002, "staff"),
        Some("alice".to_string())
    );
    super::super::plan::note_primary(
        "alice",
        super::super::plan::PlannedPrimary {
            name: "users".to_string(),
            gid: Some(1003),
        },
    );
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1002, "staff"),
        None,
        "the account the plan moves away holds the group no longer"
    );
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1003, "users"),
        Some("alice".to_string()),
        "the account holds the group the plan gives it"
    );
    // The group the plan does not know the number of yet — it is not
    // on the host, so the allocator draws the number at the apply —
    // is decided by its name.
    super::super::plan::note_primary(
        "alice",
        super::super::plan::PlannedPrimary {
            name: "users".to_string(),
            gid: None,
        },
    );
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1002, "staff"),
        None,
        "the account the plan moves away holds the group no longer"
    );
    assert_eq!(
        super::super::guards::primary_group_holder_from(passwd, 1003, "users"),
        Some("alice".to_string()),
        "the account holds the group the plan gives it"
    );
    super::super::plan::forget_primary("alice");
}

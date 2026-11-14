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

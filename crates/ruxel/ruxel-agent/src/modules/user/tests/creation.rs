//! The account creation's switches over a host that already has parts
//! of what the step claims, and the guards that must refuse before the
//! creation mutates anything.

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
fn the_creation_selects_the_group_the_host_already_has() {
    // `useradd -U` refuses to create a group that exists (exit 9), so
    // the host's same-name group is the created account's primary one,
    // selected with `-g` the way `useradd(8)` documents. The switch that
    // creates the group is the one the host has no group for.
    assert_eq!(
        super::super::primary_group_args(None, Some("alice")),
        vec!["-g", "alice"],
        "the group the host already has is selected, never created again"
    );
    assert_eq!(
        super::super::primary_group_args(None, None),
        vec!["-U"],
        "the group the host has none of is created explicitly"
    );
    assert_eq!(
        super::super::primary_group_args(Some("docker"), Some("alice")),
        vec!["-g", "docker"],
        "the declared group is the account's primary one"
    );
    assert_eq!(
        super::super::primary_group_args(Some("docker"), None),
        vec!["-g", "docker"],
        "the declared group is the account's primary one"
    );
}

#[test]
fn creation_reuses_the_groups_the_host_already_has() {
    // A group whose account does not exist yet: the creation claims
    // that group as the new account's primary one with `-g`, because
    // `useradd -U` refuses to create a group that exists (exit 9) and
    // would fail the account creation — `-U` is the switch only where
    // the same-name group is absent. The reused group is exclusive: a
    // shared same-name group is refused instead (see the next test).
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let name = groups
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            let group = fields.next()?;
            let _password = fields.next()?;
            let gid = fields.next()?;
            let members = fields.next().unwrap_or_default();
            let usable = !group.is_empty()
                && !group.starts_with(['+', '#', '-'])
                && gid.parse::<u64>().is_ok()
                && members.is_empty()
                && !passwd
                    .lines()
                    .any(|line| line.split(':').next() == Some(group));
            usable.then(|| group.to_string())
        })
        .expect("an exclusive group without a same-name account");
    let home = std::env::temp_dir().join(format!("ruxel-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    match run(
        &json!({"name": name, "home": home.to_str().unwrap()}),
        &apply_context(),
    ) {
        // A privileged host ran `useradd -g <name> …` for real: only
        // selecting the existing group could have succeeded, `-U`
        // failing on a group that exists.
        Ok(_) => {}
        // A host with no `useradd` fails at the spawn, carrying no
        // command line: nothing to read the switch from.
        Err(error) if !error.contains("useradd ") => {
            assert!(error.contains("useradd"), "{error}");
        }
        // A host that ran it refuses with the command line in hand:
        // the existing group must be selected with `-g`, never asked
        // to be created with `-U`.
        Err(error) => {
            assert!(
                !error.contains("-U"),
                "the existing group must not be asked to be created: {error}"
            );
            assert!(
                error.contains(&format!("-g {name}")),
                "the existing group is the account's primary one: {error}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn creation_refuses_a_shared_same_name_group() {
    // Thread 288: the host's same-name group with another member
    // belongs to that member, and the creation would join it — a home
    // created with that GID included. The check-mode creation must
    // refuse and name the declaration that adopts the shared group.
    // A host with no shared memberless-account group skips: the scan
    // is the host's own.
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let Some(name) = groups.lines().find_map(|line| {
        let mut fields = line.split(':');
        let group = fields.next()?;
        let _password = fields.next()?;
        let gid = fields.next()?;
        let members = fields.next().unwrap_or_default();
        let shared = !group.is_empty()
            && !group.starts_with(['+', '#', '-'])
            && gid.parse::<u64>().is_ok()
            && !members.is_empty()
            && !passwd
                .lines()
                .any(|line| line.split(':').next() == Some(group));
        shared.then(|| group.to_string())
    }) else {
        return;
    };
    let error = run(&json!({"name": name}), &check_context())
        .expect_err("creation must not silently reuse a shared group");
    assert!(error.contains("is shared with"), "{error}");
    assert!(error.contains("declare group"), "{error}");
}

#[test]
fn a_refused_identity_change_plans_no_group() {
    // The superuser's account: the identity guard refuses the change,
    // and the refusal must come before the private group the step
    // claims is created — a group created first stays behind as an
    // orphan after the task fails, and the task's own report says the
    // account was not touched.
    let current = super::super::PasswdEntry {
        uid: 0,
        gid: 0,
        ..Default::default()
    };
    let error = super::super::apply::mutations("root", &current, true, true, None, &["-u", "1"])
        .expect_err("the superuser's identity is the host's to keep");
    assert!(
        error.contains("refusing to change the identity"),
        "the guard must refuse before any group is created: {error}"
    );
}

#[test]
fn the_private_group_is_created_before_the_account_change() {
    // The group's own creation runs before `usermod`: the account's
    // supplementary list is resolved against the host, and the list
    // may name the group the step claims.
    let current = super::super::PasswdEntry {
        uid: 1000,
        gid: 1000,
        ..Default::default()
    };
    let mutations =
        super::super::apply::mutations("alice", &current, false, true, None, &["-G", "alice"])
            .expect("an allowed change plans its mutations");
    assert_eq!(
        mutations,
        vec![
            super::super::apply::Mutation::GroupAdd("alice".to_string()),
            super::super::apply::Mutation::UserMod(vec!["-G".to_string(), "alice".to_string()]),
        ],
        "the group the step claims exists before the account's own change"
    );
}

#[test]
fn the_uid_holder_is_the_account_that_carries_the_number() {
    // The live passwd database decides which numbers are taken: the
    // account that carries the number is the one a request for it
    // collides with, and an account's own number collides with
    // nothing.
    let passwd = "root:x:0:0:root:/root:/bin/sh\nalice:x:1001:1001::/home/alice:/bin/sh\nbob:x:1002:1002::/home/bob:/bin/sh\n";
    assert_eq!(
        super::super::uid_holders_from(passwd, 1002, "alice"),
        vec!["bob".to_string()]
    );
    assert_eq!(
        super::super::uid_holders_from(passwd, 1001, "alice"),
        Vec::<String>::new()
    );
    assert_eq!(
        super::super::uid_holders_from(passwd, 1001, "bob"),
        vec!["alice".to_string()]
    );
    let groups = "root:x:0:\nalice:x:1001:\nbob:x:1002:\n";
    assert_eq!(
        super::super::gid_holders_from(groups, 1002, "alice"),
        vec!["bob".to_string()]
    );
    assert_eq!(
        super::super::gid_holders_from(groups, 1001, "alice"),
        Vec::<String>::new()
    );
}

#[test]
fn a_requested_uid_another_account_holds_is_refused() {
    // The number is the host's to keep: `useradd -u` and `usermod -u`
    // refuse a number another account holds, so the plan must refuse
    // it too — the dry run may not report a change that cannot
    // succeed.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let (_name, uid) = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
        })
        .expect("a non-root account on the host");
    // The creation of another account with that number.
    let other = format!("ruxel-uid-collision-{}", std::process::id());
    let error = run(&json!({"name": other, "uid": uid}), &check_context())
        .expect_err("a uid another account holds must be refused");
    assert!(error.contains("holds that number"), "{error}");
    // The change of an existing account to a number another holds.
    let accounts: Vec<(String, u64)> = host
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
        })
        .collect();
    if let (Some((first, _)), Some((_, second_uid))) = (accounts.first(), accounts.get(1)) {
        let error = run(&json!({"name": first, "uid": second_uid}), &check_context())
            .expect_err("a uid another account holds must be refused");
        assert!(error.contains("holds that number"), "{error}");
    }
    // The group's own number: `groupadd -g` refuses a number another
    // group holds.
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    let (gid, _holder) = groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid != 0))
                .then(|| (f[2].parse::<u64>().unwrap(), f[0].to_string()))
        })
        .expect("a group with a number on the host");
    let error = group(
        &json!({"name": format!("ruxel-gid-collision-{}", std::process::id()), "gid": gid}),
        &check_context(),
    )
    .expect_err("a gid another group holds must be refused");
    assert!(error.contains("holds that number"), "{error}");
}

/// A uid no live account holds: the plan's own, fitting the request
/// the creation records.
fn free_account_uid() -> u64 {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    (60000u64..61000)
        .find(|uid| {
            !passwd.lines().any(|line| {
                line.split(':')
                    .nth(2)
                    .is_some_and(|held| held == uid.to_string())
            })
        })
        .expect("a free uid on the host")
}

#[test]
fn a_planned_creation_is_compared_not_created_again() {
    // Check mode runs no `useradd`: the first step's creation is the
    // second step's live entry. An identical declaration reports no
    // change — the apply created the account in the first stage and
    // reports the later task unchanged — instead of creating it again.
    // The plan's own record carries the requested home, shell and
    // comment, while the numbers the plan never stated stay the
    // allocator's to draw: no comparison runs against them.
    let fresh = format!("ruxel-planned-account-{}", std::process::id());
    assert!(
        passwd_entry(&fresh).unwrap().is_none(),
        "the account must not exist on the host"
    );
    let first = run(&json!({"name": fresh}), &check_context()).expect("the creation must plan");
    assert_eq!(first.get("changed"), Some(&json!(true)), "{first}");
    let again =
        run(&json!({"name": fresh}), &check_context()).expect("the redeclaration must plan");
    assert_eq!(
        again.get("changed"),
        Some(&json!(false)),
        "the planned creation compares, it does not create again: {again}"
    );
}

#[test]
fn a_numbered_creation_verifies_the_redeclaration_against_the_plan() {
    // The plan's own record carries the requested number, so a later
    // declaration stating it reports no change — the allocator draws
    // nothing for it.
    let fresh = format!("ruxel-planned-numbered-account-{}", std::process::id());
    let planned = free_account_uid();
    run(
        &json!({"name": fresh, "uid": planned, "create_home": false}),
        &check_context(),
    )
    .expect("the creation must plan");
    let again = run(
        &json!({"name": fresh, "uid": planned, "create_home": false}),
        &check_context(),
    )
    .expect("the redeclaration must plan");
    assert_eq!(
        again.get("changed"),
        Some(&json!(false)),
        "the planned number compares: {again}"
    );
}

//! The plan's own state decides what a task changes: the presence of
//! an account and of a group the plan has taken away, the passwd
//! fields an earlier task of the plan has moved, and the pre-existing
//! group a rollback must leave in place.

use super::super::*;
use std::os::unix::fs::MetadataExt;

fn check_context() -> ExecContext {
    ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

/// The live account the tests plan against, with the name of the group
/// it holds as its primary group. The account's home belongs to it
/// where `owned_home` asks for it: the home's reconciliation then
/// reports no change of its own, and the field comparison alone
/// decides what the task reports.
fn live_account(owned_home: bool) -> Option<(String, PasswdEntry, String)> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 || f[2].parse::<u64>().is_err() {
            continue;
        }
        let Ok(Some(entry)) = passwd_entry_from(&passwd, f[0]) else {
            continue;
        };
        // The account's primary group, which the step declares: an
        // account whose private group the host does not hold would
        // plan the creation of that group instead of the field the
        // test is about.
        let Some(group) = super::super::guards::group_name_of_gid_from(&groups, entry.gid) else {
            continue;
        };
        if owned_home {
            let Ok(meta) = std::fs::metadata(&entry.home) else {
                continue;
            };
            if !meta.is_dir()
                || u64::from(meta.uid()) != entry.uid
                || u64::from(meta.gid()) != entry.gid
            {
                continue;
            }
            // The home's directory must be readable: the
            // reconciliation walks it.
            if std::fs::read_dir(&entry.home).is_err() {
                continue;
            }
        }
        return Some((f[0].to_string(), entry, group));
    }
    None
}

/// A live group of the host: any group that is not the root group.
fn live_group() -> Option<String> {
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    groups.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid != 0)).then(|| f[0].to_string())
    })
}

/// A live group nothing holds: no member of it, no account that holds
/// it as its primary group, and no account of its own name.
fn live_empty_group() -> Option<String> {
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    let accounts: Vec<&str> = passwd
        .lines()
        .filter_map(|line| line.split(':').next())
        .collect();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || f[2].parse::<u64>().is_ok_and(|gid| gid == 0) {
            continue;
        }
        if !f[3].is_empty() || accounts.contains(&f[0]) {
            continue;
        }
        let gid = f[2].parse::<u64>().unwrap_or(0);
        let held = passwd.lines().any(|account| {
            let f: Vec<&str> = account.split(':').collect();
            f.len() >= 4 && f[3].parse::<u64>() == Ok(gid)
        });
        if !held {
            return Some(f[0].to_string());
        }
    }
    None
}

#[test]
fn a_rollback_moves_the_doomed_account_off_a_reused_group() {
    // A pre-existing same-name group the step reused predates the
    // step — the host's own, or an earlier plan step's — and is never
    // the rollback's to delete: `userdel` removes the same-name group
    // only where it is the account's primary group, so the rollback
    // moves the doomed account off it first, deletes the account, and
    // verifies the group survived. Nothing recreates the group: a
    // password the host's group held survives because the group is
    // never deleted, and no hash ever crosses argv.
    let reused = ("alice".to_string(), 1500u64);
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    // The simulated host: a USERGROUPS_ENAB host whose `userdel` takes
    // the same-name group only while it is the account's primary one.
    let primary = std::cell::RefCell::new(true);
    let group_present = std::cell::RefCell::new(true);
    take_back_account(
        "alice",
        Some(&reused),
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            *primary.borrow_mut() = false;
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            if *primary.borrow() {
                *group_present.borrow_mut() = false;
            }
            Ok(())
        },
        &|_name| Ok(*group_present.borrow()),
    )
    .expect("the rollback must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            "usermod -g 0 alice".to_string(),
            "userdel alice".to_string()
        ],
        "the doomed account moves off the reused group before userdel runs, and nothing recreates the group"
    );
    assert!(
        *group_present.borrow(),
        "the group the step did not create survives the rollback"
    );
}

#[test]
fn a_rollback_may_take_the_group_the_step_created() {
    // Where the host held no same-name group `useradd -U` created it,
    // so it is the step's own: the rollback takes it with the account
    // and moves nothing off first.
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    take_back_account(
        "alice",
        None,
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            Ok(())
        },
        &|_name| Ok(false),
    )
    .expect("the rollback must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec!["userdel alice".to_string()],
        "a group the step created goes with the account; nothing moves off first"
    );
}

#[test]
fn a_rollback_reports_a_reused_group_userdel_took() {
    // A host that takes the pre-existing group despite the move: the
    // rollback reports the loss instead of recreating the group
    // locked — no shadow utility restores a group password off the
    // command line, and a hash in argv is a hash every local user can
    // copy. Nothing recreates the group behind the error.
    let reused = ("alice".to_string(), 1500u64);
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let error = take_back_account(
        "alice",
        Some(&reused),
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            Ok(())
        },
        &|_name| Ok(false),
    )
    .expect_err("a group userdel took anyway must fail loudly");
    assert!(
        error.contains("alice"),
        "the error names the group the host lost: {error}"
    );
    assert_eq!(
        calls.borrow().clone(),
        vec![
            "usermod -g 0 alice".to_string(),
            "userdel alice".to_string()
        ],
        "nothing recreates the group behind the error: {calls:?}"
    );
}

#[test]
fn an_account_removal_records_the_private_group_it_takes_with_it() {
    // The account's private group is the group the account holds as
    // its primary group, and nothing else holds it: `userdel` deletes
    // the empty same-name group with the account, so the plan's
    // record says so — a later step that declares the group absent
    // reports the no-op the apply performs, the group being gone
    // before that step runs.
    let group = live_empty_group().expect("a live group nothing holds");
    let ctx = check_context();
    // The plan's account: the group is the private group of the
    // account the plan creates.
    run(&json!({"name": group, "create_home": false}), &ctx).expect("the creation must plan");
    let removal =
        run(&json!({"name": group, "state": "absent"}), &ctx).expect("the removal must plan");
    assert_eq!(
        removal.get("changed"),
        Some(&json!(true)),
        "the removal of the plan's own account must plan: {removal}"
    );
    let later = super::super::group(&json!({"name": group, "state": "absent"}), &ctx)
        .expect("the later group step must plan");
    assert_eq!(
        later.get("changed"),
        Some(&json!(false)),
        "the group is gone with the account: {later}"
    );
}

#[test]
fn a_recreated_account_is_planned_as_a_creation() {
    // The plan takes the account away and a later step declares it
    // present again: check mode runs no `userdel`, so the live passwd
    // entry is stale — the account is gone the way the apply takes it
    // — and the step that recreates it must report the creation, not
    // the modification of an entry the host no longer holds.
    let (name, _entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    // The plan's removal of the account, recorded the way the removal
    // step records it.
    super::super::plan::note_removed_account(&name);
    let result =
        run(&json!({"name": name, "group": group}), &ctx).expect("the recreation must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the account is created anew, not modified: {result}"
    );
}

#[test]
fn a_recreated_group_is_planned_as_a_creation() {
    // The group counterpart: the plan takes the group away and a
    // later step declares it present again, so the step must report
    // the creation the apply performs — `groupadd` — and not the
    // modification of the entry the host no longer holds.
    let name = live_group().expect("a live group on the host");
    let ctx = check_context();
    super::super::plan::note_removed_group(&name);
    let result =
        super::super::group(&json!({"name": name}), &ctx).expect("the recreation must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the group is created anew, not modified: {result}"
    );
}

#[test]
fn a_later_step_compares_the_home_against_the_planned_state() {
    // The live account's home, an earlier task of the plan moves it,
    // a later task moves it back: check mode runs no `usermod`, so the
    // live entry still holds the old home — the later task compares
    // its own declaration against the state the plan ends up with,
    // not against the host's unchanged one.
    let (name, entry, group) = live_account(true).expect("a live account on the host");
    let ctx = check_context();
    // The earlier task's destination lives under the operator's own
    // tree: a missing home is created beneath its ancestors, so the
    // plan admits the creation only under trusted ones — the live
    // home's own parent may belong to another principal.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-plan-moved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    let moved = format!("{}/ruxel-plan-moved", anchor.display());
    run(&json!({"name": name, "group": group, "home": moved}), &ctx)
        .expect("the earlier move must plan");
    let back = run(
        &json!({"name": name, "group": group, "home": entry.home.clone()}),
        &ctx,
    )
    .expect("the move back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the home moves twice: {back}"
    );
    std::fs::remove_dir_all(&anchor).unwrap();
}

#[test]
fn a_later_step_compares_the_shell_against_the_planned_state() {
    // The shell counterpart: the same stale-state structure as the
    // home's.
    let (name, entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    let moved = if entry.shell == "/bin/sh" {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    run(&json!({"name": name, "group": group, "shell": moved}), &ctx)
        .expect("the earlier change must plan");
    let back = run(
        &json!({"name": name, "group": group, "shell": entry.shell.clone()}),
        &ctx,
    )
    .expect("the change back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the shell moves twice: {back}"
    );
}

#[test]
fn a_later_step_compares_the_comment_against_the_planned_state() {
    // The comment counterpart: the same stale-state structure as the
    // home's.
    let (name, entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    let moved = format!("ruxel-plan-{}", entry.comment);
    run(
        &json!({"name": name, "group": group, "comment": moved}),
        &ctx,
    )
    .expect("the earlier change must plan");
    let back = run(
        &json!({"name": name, "group": group, "comment": entry.comment.clone()}),
        &ctx,
    )
    .expect("the change back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the comment moves twice: {back}"
    );
}

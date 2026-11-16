//! The plan's own state decides what a task changes: the presence of
//! an account and of a group the plan has taken away, the passwd
//! fields an earlier task of the plan has moved, and the pre-existing
//! group a rollback must leave in place.

use super::super::*;
use std::os::unix::fs::MetadataExt;

pub(super) fn check_context() -> ExecContext {
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
pub(super) fn live_account(owned_home: bool) -> Option<(String, PasswdEntry, String)> {
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
pub(super) fn live_group() -> Option<String> {
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
        PrivateGroup::Reused(&reused),
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
        &|group| {
            calls.borrow_mut().push(format!("groupdel {group}"));
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
    // so it is the step's own: the rollback moves the doomed account
    // off it first — `userdel` removes the same-name group only where
    // it is the account's primary group — deletes the account, deletes
    // the created group explicitly, and verifies it is gone. The
    // explicit `groupdel` removes it on every host policy: a host that
    // keeps empty groups (`USERGROUPS_ENAB=no`) would otherwise leave
    // the created group and its gid behind for a retry to silently
    // reuse.
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    // The simulated host: `userdel` takes the same-name group only
    // while it is the account's primary one.
    let primary = std::cell::RefCell::new(true);
    let group_present = std::cell::RefCell::new(true);
    take_back_account(
        "alice",
        PrivateGroup::Created,
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
        &|group| {
            calls.borrow_mut().push(format!("groupdel {group}"));
            *group_present.borrow_mut() = false;
            Ok(())
        },
        &|_name| Ok(*group_present.borrow()),
    )
    .expect("the rollback must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            "usermod -g 0 alice".to_string(),
            "userdel alice".to_string(),
            "groupdel alice".to_string()
        ],
        "the created group is moved off, deleted with the account, and removed explicitly"
    );
    assert!(
        !*group_present.borrow(),
        "the group the step created is gone with the account"
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
        PrivateGroup::Reused(&reused),
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            Ok(())
        },
        &|group| {
            calls.borrow_mut().push(format!("groupdel {group}"));
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
fn a_rollback_removes_the_created_group_where_userdel_keeps_it() {
    // A host that keeps empty groups (`USERGROUPS_ENAB=no`): `userdel`
    // leaves the operation-created group behind, so the rollback deletes
    // it explicitly after moving the doomed account off it — a retry
    // must not silently reuse the partial state. The absence afterwards
    // is verified.
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    // The simulated host: a `USERGROUPS_ENAB=no` host whose `userdel`
    // never takes the same-name group.
    let group_present = std::cell::RefCell::new(true);
    take_back_account(
        "alice",
        PrivateGroup::Created,
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            Ok(())
        },
        &|group| {
            calls.borrow_mut().push(format!("groupdel {group}"));
            *group_present.borrow_mut() = false;
            Ok(())
        },
        &|_name| Ok(*group_present.borrow()),
    )
    .expect("the rollback must take the created group back");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            "usermod -g 0 alice".to_string(),
            "userdel alice".to_string(),
            "groupdel alice".to_string()
        ],
        "the doomed account moves off first and the kept group is deleted explicitly"
    );
    assert!(
        !*group_present.borrow(),
        "no partial state survives for a retry to reuse"
    );
}

#[test]
fn a_rollback_reports_a_created_group_left_behind() {
    // `groupdel` reported success but the group survives: the rollback
    // says so loudly instead of handing a retry the partial state to
    // silently reuse.
    let calls: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let error = take_back_account(
        "alice",
        PrivateGroup::Created,
        &|account| {
            calls.borrow_mut().push(format!("usermod -g 0 {account}"));
            Ok(())
        },
        &|account| {
            calls.borrow_mut().push(format!("userdel {account}"));
            Ok(())
        },
        &|group| {
            calls.borrow_mut().push(format!("groupdel {group}"));
            Ok(())
        },
        &|_name| Ok(true),
    )
    .expect_err("a group left behind must fail loudly");
    assert!(
        error.contains("alice"),
        "the error names the group the host kept: {error}"
    );
    assert_eq!(
        calls.borrow().clone(),
        vec![
            "usermod -g 0 alice".to_string(),
            "userdel alice".to_string(),
            "groupdel alice".to_string()
        ],
        "nothing proceeds past the group left behind: {calls:?}"
    );
}

#[test]
fn an_account_removal_records_the_private_group_it_takes_with_it() {
    // The account's private group is the group the account holds as
    // its primary group, and nothing else holds it: on a host that
    // deletes one (`USERGROUPS_ENAB`) `userdel` deletes the empty
    // same-name group with the account, so the plan's record says so —
    // a later step that declares the group absent reports the no-op
    // the apply performs, the group being gone before that step runs.
    // On a host that keeps the empty group the plan records no removal
    // for it, so the later step reports the removal the apply runs.
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
    if super::super::holders::usergroups_enabled() {
        assert_eq!(
            later.get("changed"),
            Some(&json!(false)),
            "the group is gone with the account: {later}"
        );
    } else {
        assert_eq!(
            later.get("changed"),
            Some(&json!(true)),
            "the host keeps the empty group, so the later removal still plans: {later}"
        );
    }
}

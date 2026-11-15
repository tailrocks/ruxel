//! The home's repair and the account's own change: the repair runs
//! first, so a repair that fails leaves the account as it was.

use super::super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// An account on the host and the group it holds as its primary one.
fn an_account_with_its_group() -> (String, String) {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid = f[2].parse::<u64>().ok()?;
            if uid == 0 {
                return None;
            }
            let gid = f[3].parse::<u64>().ok()?;
            let group = groups.lines().find_map(|other| {
                let g: Vec<&str> = other.split(':').collect();
                (g.get(2).and_then(|n| n.parse::<u64>().ok()) == Some(gid))
                    .then(|| g[0].to_string())
            })?;
            Some((f[0].to_string(), group))
        })
        .expect("an account with a primary group on the host")
}

#[test]
fn a_home_repair_that_fails_leaves_the_account_unchanged() {
    // The account's own change — the passwd home and the uid bundled
    // with it — must not run before the home's repair: a repair that
    // fails leaves the account mutated, with its home field moved
    // onto a home that does not exist and its identity changed. The
    // repair runs first: it is the preflight of the account's own
    // change.
    let (account, group) = an_account_with_its_group();
    let before = passwd_entry(&account)
        .unwrap()
        .expect("the account on the host");
    let home = std::env::temp_dir().join(format!("ruxel-home-repair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The declared identity does not fit the host's numbers, so the
    // directory's owner cannot be the account's: the repair fails.
    let error = run(
        &json!({
            "name": account,
            "group": group,
            "home": home.to_str().unwrap(),
            "uid": 4_294_967_296_u64,
        }),
        &ctx,
    )
    .expect_err("the home's repair must fail the task");
    assert!(
        !error.contains("usermod"),
        "the account's own change must not run before the home's repair: {error}"
    );
    assert!(error.contains("home"), "{error}");
    let after = passwd_entry(&account)
        .unwrap()
        .expect("the account on the host");
    assert_eq!(after.uid, before.uid, "the account's uid must not change");
    assert_eq!(after.gid, before.gid, "the account's gid must not change");
    assert_eq!(after.home, before.home, "the account's home must not move");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn the_declared_home_repair_chowns_to_the_declared_group() {
    // The repair runs against the identity the plan ends up with: the
    // declared group's live number, never the account's old one.
    let (account, primary) = an_account_with_its_group();
    let (_, gid) = group_entry(&primary).unwrap().expect("the live group");
    let repair = super::super::planned_home_repair(
        Some("/srv/alice"),
        "/srv/alice",
        &account,
        Some(&primary),
        Some(1500),
        1500,
        999,
        false,
        false,
        true,
        true,
    )
    .expect("the owner must resolve");
    assert_eq!(
        repair,
        Some(super::super::apply::Home {
            path: "/srv/alice".to_string(),
            uid: 1500,
            owner: super::super::apply::HomeOwner::Number(gid),
            managed_uid: 1500,
        }),
        "the declared home is repaired to the declared group's number"
    );
}

#[test]
fn the_default_home_repair_covers_a_primary_group_move() {
    // A primary-group renumbering (`groupmod -g`) leaves the home
    // directory's ownership behind, so a step that moves the group
    // without declaring a home still repairs the effective default
    // home — while a step that moves no identity manages the untouched
    // default nowhere.
    let (account, primary) = an_account_with_its_group();
    let (_, gid) = group_entry(&primary).unwrap().expect("the live group");
    let repair = super::super::planned_home_repair(
        None,
        &format!("/home/{account}"),
        &account,
        Some(&primary),
        None,
        1500,
        999,
        false,
        true,
        true,
        true,
    )
    .expect("the owner must resolve");
    assert_eq!(
        repair,
        Some(super::super::apply::Home {
            path: format!("/home/{account}"),
            uid: 1500,
            owner: super::super::apply::HomeOwner::Number(gid),
            managed_uid: 1500,
        }),
        "the default home is repaired to the moved group's number"
    );
    // No move, no repair: a no-op task never claimed the default home.
    let idle = super::super::planned_home_repair(
        None,
        &format!("/home/{account}"),
        &account,
        Some(&primary),
        None,
        1500,
        gid,
        false,
        false,
        true,
        true,
    )
    .expect("the idle step plans");
    assert_eq!(idle, None, "a step that moves no identity repairs nothing");
}

#[test]
fn no_repair_runs_against_numbers_the_allocator_draws() {
    // Numbers the plan never stated are the allocator's to draw: the
    // repair runs against none of them — the primary-group change the
    // plan already reports covers it. An explicit home with unknown
    // numbers plans no repair either.
    let (account, _) = an_account_with_its_group();
    let unknown = super::super::planned_home_repair(
        None,
        "/srv/alice",
        &account,
        None,
        None,
        u64::MAX,
        u64::MAX,
        true,
        true,
        false,
        true,
    )
    .expect("the unknown numbers plan no repair");
    assert_eq!(
        unknown, None,
        "no repair runs against allocator-drawn numbers"
    );
    let explicit = super::super::planned_home_repair(
        Some("/srv/alice"),
        "/srv/alice",
        &account,
        None,
        None,
        u64::MAX,
        u64::MAX,
        true,
        true,
        false,
        true,
    )
    .expect("the unknown numbers plan no repair");
    assert_eq!(
        explicit, None,
        "no repair runs against allocator-drawn numbers"
    );
}

#[test]
fn a_missing_group_plans_no_repair_for_the_change_it_covers() {
    // A group that is not on the host yet — check mode runs no
    // `groupadd`, so the plan's own creation draws its number at the
    // apply — carries no gid to chown to: the primary-group change the
    // plan already reports covers the repair.
    let (account, _) = an_account_with_its_group();
    let missing = format!("ruxel-missing-repair-group-{}", std::process::id());
    assert!(
        group_entry(&missing).unwrap().is_none(),
        "the group must not exist on the host"
    );
    super::super::entries::note_planned_group(&missing, None);
    let repair = super::super::planned_home_repair(
        None,
        &format!("/home/{account}"),
        &account,
        Some(&missing),
        None,
        1500,
        999,
        false,
        true,
        true,
        true,
    )
    .expect("the pending group plans no repair");
    assert_eq!(repair, None, "a group with no number yet repairs nothing");
    super::super::entries::forget_planned_group(&missing);
}

#[test]
fn a_group_only_change_repairs_the_account_effective_home() {
    // An existing account with a custom passwd home (`/srv/alice`)
    // that changes only its primary group: the repair covers the home
    // the account holds under the plan — not the creation default
    // `/home/<name>`, a path this step never claimed and must not
    // create or chown while the real home keeps the old GID.
    let (account, primary) = an_account_with_its_group();
    let (_, gid) = group_entry(&primary).unwrap().expect("the live group");
    let repair = super::super::planned_home_repair(
        None,
        "/srv/alice",
        &account,
        Some(&primary),
        None,
        1500,
        999,
        false,
        true,
        true,
        true,
    )
    .expect("the owner must resolve");
    assert_eq!(
        repair,
        Some(super::super::apply::Home {
            path: "/srv/alice".to_string(),
            uid: 1500,
            owner: super::super::apply::HomeOwner::Number(gid),
            managed_uid: 1500,
        }),
        "the group-only change repairs the account's effective home, not the creation default"
    );
    // An account whose effective home is the default still repairs
    // that default: the effective home is what decides, either way.
    let default = super::super::planned_home_repair(
        None,
        &format!("/home/{account}"),
        &account,
        Some(&primary),
        None,
        1500,
        999,
        false,
        true,
        true,
        true,
    )
    .expect("the owner must resolve");
    assert_eq!(
        default,
        Some(super::super::apply::Home {
            path: format!("/home/{account}"),
            uid: 1500,
            owner: super::super::apply::HomeOwner::Number(gid),
            managed_uid: 1500,
        }),
        "a default effective home still repairs the default path"
    );
}

#[test]
fn a_failed_creation_takes_back_only_the_home_it_created() {
    // `useradd -m` created the home where it was missing: the rollback
    // of the failed reconciliation removes exactly what the operation
    // added — while a home that pre-existed is the host's, whatever it
    // holds. The home's repair and its rollback live a layer below
    // (`apply::run` takes back the repairs of an existing account's
    // change): this one covers the creation's own home only, so the two
    // never take back the same directory.
    let take_back = |home: &std::path::Path, preexisted: bool, calls: Rc<RefCell<Vec<String>>>| {
        super::super::take_back_created_account(
            "alice",
            super::super::PrivateGroup::Declared,
            home.to_str().unwrap(),
            preexisted,
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
            &|_: &str| Ok(true),
        )
    };
    // The operation created the home: it is taken back with the account.
    let created = std::env::temp_dir().join(format!("ruxel-created-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&created);
    std::fs::create_dir_all(&created).unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    take_back(&created, false, Rc::clone(&calls)).expect("the take-back must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec!["userdel alice".to_string()],
        "the account is taken back"
    );
    assert!(
        !created.exists(),
        "the home the operation created is taken back"
    );
    // The home pre-existed: it survives with its contents.
    let kept = std::env::temp_dir().join(format!("ruxel-kept-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&kept);
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("keep"), b"keep").unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    take_back(&kept, true, Rc::clone(&calls)).expect("the take-back must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec!["userdel alice".to_string()],
        "the account is taken back"
    );
    assert_eq!(
        std::fs::read(kept.join("keep")).unwrap(),
        b"keep",
        "the pre-existing home survives with its contents"
    );
    // Nothing was created and nothing is there: only the account goes.
    let missing = std::env::temp_dir().join(format!("ruxel-missing-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let calls = Rc::new(RefCell::new(Vec::new()));
    take_back(&missing, false, Rc::clone(&calls)).expect("the take-back must succeed");
    assert_eq!(
        calls.borrow().clone(),
        vec!["userdel alice".to_string()],
        "the account is taken back"
    );
    // A created home another writer filled is kept and reported: the
    // removal refuses a directory that is not empty, the way the
    // home-repair rollback does.
    let filled = std::env::temp_dir().join(format!("ruxel-filled-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&filled);
    std::fs::create_dir_all(&filled).unwrap();
    std::fs::write(filled.join("planted"), b"planted").unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let error = take_back(&filled, false, Rc::clone(&calls))
        .expect_err("a non-empty home must not be deleted");
    assert!(error.contains("take back created home"), "{error}");
    assert_eq!(
        calls.borrow().clone(),
        vec!["userdel alice".to_string()],
        "the account was taken back before the home refused"
    );
    assert!(
        filled.join("planted").exists(),
        "the planted contents are kept and reported"
    );
    std::fs::remove_dir_all(&kept).unwrap();
    std::fs::remove_dir_all(&filled).unwrap();
}

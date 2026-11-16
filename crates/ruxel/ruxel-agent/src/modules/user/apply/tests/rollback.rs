use super::*;
use std::cell::RefCell;

/// The identity the test process owns: a probe file of it, so no
/// ownership change is needed where the repair targets it.
fn own_identity(scenario: &str) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let probe = std::env::temp_dir().join(format!(
        "ruxel-rollback-probe-{scenario}-{}",
        std::process::id()
    ));
    std::fs::write(&probe, b"").unwrap();
    let meta = std::fs::metadata(&probe).unwrap();
    let identity = (u64::from(meta.uid()), u64::from(meta.gid()));
    std::fs::remove_file(&probe).unwrap();
    identity
}

fn home_mutation(path: &std::path::Path, uid: u64, gid: u64) -> Mutation {
    Mutation::Home(Home {
        path: path.to_str().unwrap().to_string(),
        uid,
        owner: HomeOwner::Number(gid),
        managed_uid: uid,
    })
}

#[test]
fn a_failed_home_repair_takes_back_the_created_group() {
    // The group's own creation runs before the home's repair, and
    // the repair can fail — an immutable directory, a read-only
    // parent, a path under a regular file. The group the step
    // created must not stay behind when the task reports failure.
    let file = std::env::temp_dir().join(format!("ruxel-rollback-file-{}", std::process::id()));
    std::fs::write(&file, b"not a directory").unwrap();
    let home = file.join("home");
    let group = format!("ruxel-rollback-group-{}", std::process::id());
    let mutations = vec![
        Mutation::GroupAdd(group.clone()),
        home_mutation(&home, 1500, 1500),
    ];
    let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
    let error = run("ruxel-rollback-alice", mutations, false, &{
        let calls = std::rc::Rc::clone(&calls);
        move |cmd: &str, args: &[&str]| {
            calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
            Ok(())
        }
    })
    .expect_err("the home's repair must fail the task");
    assert!(error.contains("home"), "{error}");
    assert_eq!(
        calls.borrow().clone(),
        vec![format!("groupadd {group}"), format!("groupdel {group}")],
        "the created group is taken back: {error}"
    );
    std::fs::remove_file(&file).unwrap();
}

#[test]
fn a_failed_account_change_takes_back_the_home_and_the_group() {
    // The home's repair runs before the account's own change, and
    // the change can fail after the repair created the directory.
    // The created directory and the created group must both go back
    // with the failure. The missing home is created beneath its
    // ancestors, so the case stages it under the operator's own tree:
    // a shared-writable ancestor would refuse the creation before any
    // mutation.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-rollback-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir_all(&anchor).unwrap();
    let base = anchor;
    let home = base.join("alice");
    let (uid, gid) = own_identity("created");
    let group = format!("ruxel-rollback-group-{}", std::process::id());
    let mutations = vec![
        Mutation::GroupAdd(group.clone()),
        home_mutation(&home, uid, gid),
        Mutation::UserMod(vec!["-s".to_string(), "/bin/sh".to_string()]),
    ];
    let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
    let error = run("ruxel-rollback-alice", mutations, false, &{
        let calls = std::rc::Rc::clone(&calls);
        move |cmd: &str, args: &[&str]| {
            calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
            if cmd == "usermod" {
                return Err("injected usermod failure".to_string());
            }
            Ok(())
        }
    })
    .expect_err("the account's change must fail the task");
    assert!(error.contains("injected usermod failure"), "{error}");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            format!("groupadd {group}"),
            "usermod -s /bin/sh ruxel-rollback-alice".to_string(),
            format!("groupdel {group}"),
        ],
        "the repair is reversed before the group: {error}"
    );
    assert!(
        !home.exists(),
        "the directory the repair created is taken back"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_failed_account_change_takes_back_every_parent_the_repair_created() {
    // An explicit home with several missing components: the repair
    // creates the whole chain before the account's own change, and the
    // change can fail after that. The rollback removes every parent
    // the repair created, leaf first — a failed task must not leave
    // the newly created directories behind. The missing home is
    // created beneath its ancestors, so the case stages it under the
    // operator's own tree.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-rollback-parents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir_all(&anchor).unwrap();
    let home = anchor.join("srv/accounts/alice");
    let parents = [anchor.join("srv"), anchor.join("srv/accounts")];
    let (uid, gid) = own_identity("parents");
    let group = format!("ruxel-rollback-group-{}", std::process::id());
    let mutations = vec![
        Mutation::GroupAdd(group.clone()),
        home_mutation(&home, uid, gid),
        Mutation::UserMod(vec!["-s".to_string(), "/bin/sh".to_string()]),
    ];
    let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
    let error = run("ruxel-rollback-alice", mutations, false, &{
        let calls = std::rc::Rc::clone(&calls);
        move |cmd: &str, args: &[&str]| {
            calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
            if cmd == "usermod" {
                return Err("injected usermod failure".to_string());
            }
            Ok(())
        }
    })
    .expect_err("the account's change must fail the task");
    assert!(error.contains("injected usermod failure"), "{error}");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            format!("groupadd {group}"),
            "usermod -s /bin/sh ruxel-rollback-alice".to_string(),
            format!("groupdel {group}"),
        ],
        "the repair is reversed before the group: {error}"
    );
    assert!(
        !home.exists(),
        "the directory the repair created is taken back"
    );
    for parent in &parents {
        assert!(
            !parent.exists(),
            "every parent the repair created is taken back: {}",
            parent.display()
        );
    }
    assert!(anchor.is_dir(), "the ancestor that already existed is kept");
    std::fs::remove_dir_all(&anchor).unwrap();
}

#[test]
fn a_failed_repair_of_an_existing_home_restores_its_ownership() {
    // The repair of a home that exists keeps the directory and only
    // changes its owner; the rollback restores the ownership the
    // repair found. The directory already belongs to the test
    // process, so no ownership change is needed unprivileged — the
    // rollback runs the same path and keeps the directory.
    let base = std::env::temp_dir().join(format!("ruxel-rollback-kept-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let (uid, gid) = own_identity("kept");
    let group = format!("ruxel-rollback-group-{}", std::process::id());
    let mutations = vec![
        Mutation::GroupAdd(group.clone()),
        home_mutation(&base, uid, gid),
        Mutation::UserMod(vec!["-s".to_string(), "/bin/sh".to_string()]),
    ];
    let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
    let error = run("ruxel-rollback-alice", mutations, false, &{
        let calls = std::rc::Rc::clone(&calls);
        move |cmd: &str, args: &[&str]| {
            calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
            if cmd == "usermod" {
                return Err("injected usermod failure".to_string());
            }
            Ok(())
        }
    })
    .expect_err("the account's change must fail the task");
    assert!(error.contains("injected usermod failure"), "{error}");
    assert_eq!(
        calls.borrow().clone(),
        vec![
            format!("groupadd {group}"),
            "usermod -s /bin/sh ruxel-rollback-alice".to_string(),
            format!("groupdel {group}"),
        ],
        "the existing home is restored before the group: {error}"
    );
    assert!(base.is_dir(), "the existing home is kept, only restored");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_created_home_with_only_skeleton_contents_is_taken_back() {
    // `useradd -m` populates the new home from a non-empty `/etc/skel`:
    // the rollback of a failed reconciliation must take the
    // operation-created home back with those skeleton contents — the old
    // `remove_dir` path failed with `Directory not empty` and left the
    // home behind. A home that holds only skeleton entries is removed
    // recursively.
    let scenario = format!("ruxel-created-skel-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(skel.join("docs")).unwrap();
    std::fs::write(skel.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::write(skel.join("docs/readme"), b"skeleton\n").unwrap();
    // The home `useradd -m` made: the skeleton's entries, copied.
    std::fs::create_dir_all(home.join("docs")).unwrap();
    std::fs::write(home.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::write(home.join("docs/readme"), b"skeleton\n").unwrap();
    super::rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect("skeleton contents must be taken back");
    assert!(!home.exists(), "the skeleton-populated home is taken back");
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_created_home_with_planted_contents_is_kept_and_reported() {
    // The foreign-content refusal survives the skeleton accounting: a
    // concurrent writer's entry that the skeleton never held is kept and
    // reported rather than deleted, even where the home also holds
    // skeleton contents.
    let scenario = format!("ruxel-created-planted-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&skel).unwrap();
    std::fs::write(skel.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::write(home.join("planted"), b"planted\n").unwrap();
    let error = super::rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect_err("a planted entry must not be deleted");
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert!(
        home.join("planted").exists(),
        "the planted contents are kept and reported"
    );
    assert!(
        home.join(".bashrc").exists(),
        "nothing is deleted behind the refusal"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_failed_rollback_is_reported_with_the_original_error() {
    // The rollback itself can fail — the group is already gone, the
    // directory is held open. The task then reports both errors: the
    // host keeps the partial change, and the report says so instead
    // of claiming a clean refusal.
    let file = std::env::temp_dir().join(format!("ruxel-rollback-doomed-{}", std::process::id()));
    std::fs::write(&file, b"not a directory").unwrap();
    let home = file.join("home");
    let group = format!("ruxel-rollback-group-{}", std::process::id());
    let mutations = vec![
        Mutation::GroupAdd(group.clone()),
        home_mutation(&home, 1500, 1500),
    ];
    let error = run("ruxel-rollback-alice", mutations, false, &|cmd, _| {
        if cmd == "groupdel" {
            return Err("injected groupdel failure".to_string());
        }
        Ok(())
    })
    .expect_err("the home's repair must fail the task");
    assert!(error.contains("home"), "{error}");
    assert!(
        error.contains("could not be taken back") && error.contains("injected groupdel failure"),
        "the rollback failure is reported with the original error: {error}"
    );
    std::fs::remove_file(&file).unwrap();
}

#[test]
fn a_home_repair_that_fails_after_creating_parents_takes_them_back() {
    // The repair creates missing parents before it fails — here the
    // ownership change a non-root repair cannot perform — and the
    // failure must take the partial hierarchy back: the rollback
    // reverts the earlier mutations and removes the created parents
    // instead of leaving them behind. The missing home is created
    // beneath its ancestors, so the case stages it under the
    // operator's own tree. Only the unprivileged lanes fail the
    // ownership change, so the root lane skips it.
    if super::super::home::operating_uid() == 0 {
        return;
    }
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-rollback-partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir_all(&anchor).unwrap();
    let home = anchor.join("stage/alice");
    let stranger = 65534u64;
    let error = run(
        "ruxel-rollback-alice",
        vec![home_mutation(&home, stranger, stranger)],
        false,
        &|_, _| Ok(()),
    )
    .expect_err("the ownership change a non-root repair cannot perform must fail");
    assert!(error.contains("home"), "{error}");
    assert!(
        !home.exists() && !anchor.join("stage").exists(),
        "the partial hierarchy the failed repair created is taken back"
    );
    assert!(
        anchor.is_dir(),
        "the pre-existing anchor survives the rollback"
    );
    std::fs::remove_dir_all(&anchor).unwrap();
}

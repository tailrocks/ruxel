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

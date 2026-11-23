//! The home path the account's step manages before it touches the
//! account: every component of it, declared or claimed.

use super::super::*;

type Managed = super::super::home::ManagedAccount<'static>;

/// The repair and the preflight the tests drive: the account's number
/// is unknown, so no intended claim can match it.
fn managed() -> Managed {
    super::super::home::ManagedAccount::unnumbered("test")
}

fn repair(home: &str, uid: u64, gid: u64, check: bool) -> Result<bool, String> {
    super::super::ensure_home(home, uid, gid, check, &managed(), &mut Vec::new())
}

fn usable(home: &str) -> Result<(), String> {
    super::super::home_usable(home, &managed())
}

#[test]
fn a_home_under_a_symlinked_ancestor_is_refused_never_traversed() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let base = std::env::temp_dir().join(format!("ruxel-ancestor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let target = base.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let writable = base.join("writable");
    std::fs::create_dir(&writable).unwrap();
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o777)).unwrap();
    let link = writable.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let home = link.join("alice");
    // The walk refuses the symlinked component before any lookup
    // follows it: path-based creation and chown would build the home
    // beneath the link's target as root.
    let error = usable(home.to_str().unwrap())
        .expect_err("a home under a symlinked ancestor must be refused");
    assert!(error.contains("home"), "{error}");
    let meta = std::fs::metadata(&target).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    let error = repair(home.to_str().unwrap(), uid, gid, false)
        .expect_err("a home under a symlinked ancestor must never be created through the link");
    assert!(error.contains("home"), "{error}");
    assert!(
        !target.join("alice").exists(),
        "the home must not be created through the symlinked ancestor"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_claimed_default_home_is_refused_before_the_account_changes() {
    use std::os::unix::fs::MetadataExt;
    // The footprint's `/home/<name>` is the path the creation points
    // the new account at: a regular file there is refused before the
    // account is created, exactly like a declared one.
    let probe = std::env::temp_dir().join(format!("ruxel-claimed-home-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    let expected = std::fs::metadata(&probe).unwrap();
    // `/home` is a real directory on Linux and a symlink on macOS:
    // the account name is the one whose claimed `/home/<name>` is the
    // probe file — identified by device and inode.
    let name = [
        format!("..{}", probe.display()),
        format!("../private{}", probe.display()),
    ]
    .into_iter()
    .find(|candidate| {
        std::fs::metadata(format!("/home/{candidate}"))
            .is_ok_and(|meta| meta.dev() == expected.dev() && meta.ino() == expected.ino())
    })
    .expect("a name whose claimed home is the probe file");
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let error = run(&json!({"name": name}), &ctx).expect_err(
        "a claimed default home that is a regular file must be refused before the account changes",
    );
    assert!(error.contains("home"), "{error}");
    std::fs::remove_file(&probe).unwrap();
}

#[test]
fn an_existing_account_is_not_judged_by_an_unclaimed_default_home() {
    // The claimed default is the creation's path. An existing account
    // that states no home is pointed nowhere new, so the untouched
    // default is not this step's to validate.
    assert!(
        super::super::manages_home(false, false),
        "the creation manages the claimed default home"
    );
    assert!(
        super::super::manages_home(true, true),
        "a declared home is managed for an existing account too"
    );
    assert!(
        !super::super::manages_home(true, false),
        "an existing account's unclaimed default home is not this step's to validate"
    );
}

#[test]
fn the_declared_home_of_an_existing_account_is_still_validated() {
    // A declared home moves the existing account's field, so the
    // step validates it — a regular file there is refused.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u64>().is_ok_and(|uid| uid != 0))
                .then(|| f[0].to_string())
        })
        .expect("a non-root account on the host");
    let probe = std::env::temp_dir().join(format!("ruxel-declared-home-{}", std::process::id()));
    std::fs::write(&probe, b"not a home").unwrap();
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let error = run(
        &json!({"name": name, "home": probe.to_str().unwrap()}),
        &ctx,
    )
    .expect_err("a declared home that is a regular file must be refused");
    assert!(error.contains("home"), "{error}");
    std::fs::remove_file(&probe).unwrap();
}

#[test]
fn the_home_walk_stays_on_the_directories_it_opened() {
    // The walk opens the parent, the attacker swaps in a symlink,
    // and the mutation still lands on the inode the walk opened.
    let base = std::env::temp_dir().join(format!("ruxel-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let real = base.join("real");
    std::fs::create_dir_all(&real).unwrap();
    let target = base.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let home = real.join("home");
    let opened =
        super::super::home::open_chain(&real, home.to_str().unwrap(), false, &mut Vec::new())
            .unwrap();
    std::fs::rename(&real, base.join("real-moved")).unwrap();
    std::os::unix::fs::symlink(&target, &real).unwrap();
    rustix::fs::mkdirat(&opened, "home", rustix::fs::Mode::from_raw_mode(0o777)).unwrap();
    assert!(
        base.join("real-moved/home").is_dir(),
        "the mutation must land on the directory the walk opened"
    );
    assert!(
        !target.join("home").exists(),
        "never through the replacement the attacker planted"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_home_walk_restarts_at_the_root_for_an_absolute_symlink() {
    use std::os::unix::fs::PermissionsExt;
    // A managed home traverses a root-owned absolute symlink below
    // the first component: for an absolute target the walk restarts
    // at the root and must land on the target's directory. Only the
    // root lane can stage the admitted link, so the unprivileged
    // lanes skip it.
    if super::super::home::operating_uid() != 0 {
        // Not the root lane: the admitted-symlink scenario cannot be
        // staged unprivileged.
        return;
    }
    let base = std::path::Path::new("/").join(format!("ruxel-abs-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("mnt/homes")).unwrap();
    std::fs::create_dir_all(base.join("srv")).unwrap();
    for dir in [&base, &base.join("srv"), &base.join("mnt")] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The operator's absolute link in a root-owned unwritable directory.
    std::os::unix::fs::symlink(base.join("mnt/homes"), base.join("srv/homes")).unwrap();
    let home = base.join("srv/homes/alice");
    let changed = repair(home.to_str().unwrap(), 0, 0, false).unwrap();
    assert!(changed, "the missing home is created");
    assert!(
        base.join("mnt/homes/alice").is_dir(),
        "the walk must land on the absolute symlink's target, not under the already-walked prefix"
    );
    assert!(
        !base.join("srv/tmp").exists() && !base.join("srv/mnt").exists(),
        "the retained prefix must not be walked again: the target restarts at the root"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_home_that_is_a_regular_file_is_refused_before_the_account_changes() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let probe = std::env::temp_dir().join(format!("ruxel-home-file-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    // `usermod -d` moves the passwd field even for a regular file,
    // so the path is validated before any mutation.
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "home":probe.to_str().unwrap()}),
        &ctx,
    )
    .expect_err("a home that is a regular file must be refused before the account changes");
    assert!(error.contains("home"), "{error}");
    // The home helper refuses the same path in both modes.
    assert!(usable(probe.to_str().unwrap()).is_err());
    std::fs::remove_file(&probe).unwrap();
}

#[test]
fn an_existing_home_directory_reports_drifted_ownership() {
    use std::os::unix::fs::MetadataExt;
    // The identity the test process owns: a probe file of it.
    let dir = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-home-owned-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    // The identity the test process owns, read from the staged
    // directory itself: a probe elsewhere can carry another group.
    let meta = std::fs::metadata(&dir).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    // A home that drifted to another identity is the agent's to reconcile.
    assert!(
        repair(dir.to_str().unwrap(), uid + 1, gid + 1, true).unwrap(),
        "check mode reports the pending repair"
    );
    assert!(
        !repair(dir.to_str().unwrap(), uid, gid, false).unwrap(),
        "matching ownership is no change"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_declared_home_that_is_a_symlink_is_refused_never_chowned() {
    use std::os::unix::fs::MetadataExt;
    // A symlink to a directory would be followed by the metadata
    // probe and by chown as root: a symlink is refused and ownership
    // changes never follow one.
    let target = std::env::temp_dir().join(format!("ruxel-home-target-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir(&target).unwrap();
    let link = std::env::temp_dir().join(format!("ruxel-home-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let meta = std::fs::metadata(&target).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    assert!(usable(link.to_str().unwrap()).is_err());
    assert!(repair(link.to_str().unwrap(), uid, gid, true).is_err());
    assert!(repair(link.to_str().unwrap(), uid, gid, false).is_err());
    let after = std::fs::metadata(&target).unwrap();
    assert_eq!(
        after.uid(),
        meta.uid(),
        "the symlink target must not be chowned"
    );
    assert_eq!(
        after.gid(),
        meta.gid(),
        "the symlink target must not be chowned"
    );
    std::fs::remove_file(&link).unwrap();
    std::fs::remove_dir_all(&target).unwrap();
}

#[test]
fn a_home_lookup_error_beyond_not_found_is_refused_before_the_account_changes() {
    // A regular file on an ancestor fails `NotADirectory` — not a
    // missing home — so the lookup error propagates before any
    // mutation instead of moving the passwd field first.
    let dir = std::env::temp_dir().join(format!("ruxel-home-notdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("f");
    std::fs::write(&file, b"").unwrap();
    let nested = dir.join("f/alice");
    let error = usable(nested.to_str().unwrap()).expect_err("NotADirectory must not be usable");
    assert!(error.contains("home"), "{error}");
    // The same path through the operation is refused in both modes.
    for check_mode in [true, false] {
        let ctx = ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let error = run(
            &json!({"name":"ruxel-synthetic-user", "home":nested.to_str().unwrap()}),
            &ctx,
        )
        .expect_err("a home under a regular file must be refused before the account changes");
        assert!(error.contains("home"), "{error}");
    }
    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn a_failed_home_reconciliation_takes_the_created_account_back() {
    // `useradd` has already put the account in the passwd database
    // when the home's reconciliation runs, and the reconciliation can
    // fail — an immutable directory, a directory on a read-only
    // mount. The account the step created must be taken back then: a
    // task that fails with the account left behind leaves the host's
    // account database in a state the workflow never asked for.
    let file = std::env::temp_dir().join(format!("ruxel-not-a-directory-{}", std::process::id()));
    std::fs::write(&file, b"not a directory").unwrap();
    let home = file.join("home");
    let taken = std::cell::RefCell::new(Vec::new());
    let error = super::super::reconcile_created_home(
        "ruxel-created-account",
        home.to_str().unwrap(),
        &super::super::PasswdEntry {
            uid: 1001,
            gid: 1001,
            ..Default::default()
        },
        false,
        &|account| {
            taken.borrow_mut().push(account.to_string());
            Ok(())
        },
    )
    .expect_err("the reconciliation must fail");
    assert!(error.contains("not a directory"), "{error}");
    assert_eq!(
        taken.borrow().clone(),
        vec!["ruxel-created-account".to_string()],
        "the account the step created must be taken back"
    );
    // A removal that fails itself is reported: the host keeps the
    // account, and the task says so.
    let error = super::super::reconcile_created_home(
        "ruxel-created-account",
        home.to_str().unwrap(),
        &super::super::PasswdEntry {
            uid: 1001,
            gid: 1001,
            ..Default::default()
        },
        false,
        &|_| Err("exec userdel: no such file or directory".to_string()),
    )
    .expect_err("the reconciliation must fail");
    assert!(error.contains("could not be taken back"), "{error}");
    std::fs::remove_file(&file).unwrap();
    // A repair that creates parents before it fails takes them back
    // with the account. Only the unprivileged lanes fail the
    // ownership change, so the root lane skips this scenario.
    if super::super::home::operating_uid() != 0 {
        let base = std::env::home_dir()
            .expect("the test process runs with a home directory")
            .join(format!("ruxel-home-staged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir(&base).unwrap();
        let staged = base.join("stage/alice");
        let error = super::super::reconcile_created_home(
            "ruxel-staged-account",
            staged.to_str().unwrap(),
            &super::super::PasswdEntry {
                uid: 65534,
                gid: 65534,
                ..Default::default()
            },
            false,
            &|account| {
                taken.borrow_mut().push(account.to_string());
                Ok(())
            },
        )
        .expect_err("the failed repair must not leave its parents behind");
        assert!(error.contains("home"), "{error}");
        assert!(
            !base.join("stage").exists(),
            "the parents the failed repair created go with the account"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }
}

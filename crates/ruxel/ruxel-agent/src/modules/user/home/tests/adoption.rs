use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// A scratch directory of the test's own: unique to the test, so
/// tests running beside it cannot collide with it.
fn scratch(scenario: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-adopt-{scenario}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The identity the test process owns: a probe file of it, so the
/// tests stage the operator's own directories without privilege.
fn own_identity() -> (u64, u64) {
    let probe = std::env::temp_dir().join(format!(
        "ruxel-adopt-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&probe, b"").unwrap();
    let meta = std::fs::metadata(&probe).unwrap();
    let identity = (u64::from(meta.uid()), u64::from(meta.gid()));
    std::fs::remove_file(&probe).unwrap();
    identity
}

#[test]
fn an_attacker_writable_home_is_refused_never_adopted() {
    // The shared-writable parent (`/tmp`) with a pre-created home
    // in it: a non-root principal may have placed the directory
    // and filled it — the planted login script is the payload the
    // new account would then execute. The agent must refuse the
    // adoption in every mode instead of changing the ownership and
    // keeping the contents.
    let parent = scratch("attack");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    let (uid, gid) = own_identity();
    // The preflight refuses before any mutation.
    let error = home_usable(home.to_str().unwrap())
        .expect_err("a writable pre-created home must be refused");
    assert!(error.contains("adopt"), "{error}");
    // The repair refuses the same way in both modes.
    for check_mode in [true, false] {
        let error = ensure_home(home.to_str().unwrap(), uid, gid, check_mode)
            .expect_err("a writable pre-created home must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The creation path refuses the same way and takes the account
    // back instead of leaving it pointed at the planted directory.
    let taken: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let error = reconcile_created_home(
        "ruxel-adopt-alice",
        home.to_str().unwrap(),
        &super::super::entries::PasswdEntry {
            uid,
            gid,
            ..Default::default()
        },
        false,
        &|account| {
            taken.borrow_mut().push(account.to_string());
            Ok(())
        },
    )
    .expect_err("the creation must not adopt the planted home");
    assert!(error.contains("adopt"), "{error}");
    assert_eq!(
        taken.borrow().clone(),
        vec!["ruxel-adopt-alice".to_string()],
        "the account the step created is taken back"
    );
    // Nothing was adopted: the ownership and the planted contents
    // are exactly what the principal left.
    let meta = std::fs::metadata(&home).unwrap();
    assert_eq!(
        (u64::from(meta.uid()), u64::from(meta.gid())),
        (uid, gid),
        "the planted directory keeps its owner"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"evil\n",
        "the planted contents are preserved, never handed over"
    );
    std::fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn an_operator_placed_home_is_adopted() {
    // The operator's own staging under a shared path: the
    // directory the operator placed — owned by the operating
    // principal and writable by no one else — is adopted, with its
    // contents. `useradd -m` leaves a pre-existing directory's
    // ownership untouched, so the reconciliation repairs it with
    // the account's final identity instead of refusing it.
    let parent = scratch("operator");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
    let (uid, gid) = own_identity();
    // The preflight admits the operator's staging.
    home_usable(home.to_str().unwrap()).expect("the operator's home must be usable");
    // Check mode reports the repair where the ownership drifted.
    assert!(
        ensure_home(home.to_str().unwrap(), uid + 1, gid + 1, true).unwrap(),
        "the drifted operator home reports its repair"
    );
    // Apply mode with matching ownership is no change — accepted,
    // nothing to do — and keeps the skeleton.
    assert!(
        !ensure_home(home.to_str().unwrap(), uid, gid, false).unwrap(),
        "the operator's home is adopted, not refused"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"operator skeleton\n",
        "the operator's contents survive the adoption"
    );
    // The creation path adopts it the same way.
    let error: Result<bool, String> = reconcile_created_home(
        "ruxel-adopt-alice",
        home.to_str().unwrap(),
        &super::super::entries::PasswdEntry {
            uid,
            gid,
            ..Default::default()
        },
        false,
        &|_| panic!("nothing to take back: the adoption succeeds"),
    );
    assert!(error.is_ok(), "the creation adopts the operator's home");
    std::fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn an_attacker_owned_home_under_a_shared_path_is_refused() {
    // The finding's exact shape: the declared home pre-created by
    // another uid under a shared-writable path. Only the root lane
    // can stage another uid's directory, so the unprivileged lanes
    // skip it; the root lane proves the refusal the pre-created
    // directory earns whatever its mode is.
    if super::operating_uid() != 0 {
        return;
    }
    let parent = scratch("attack-owned");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    // Another principal's directory: the nobody account's numbers.
    let nobody = 65534u32;
    rustix::fs::chown(
        &home,
        Some(rustix::fs::Uid::from_raw(nobody)),
        Some(rustix::fs::Gid::from_raw(nobody)),
    )
    .unwrap();
    let error = home_usable(home.to_str().unwrap())
        .expect_err("another principal's pre-created home must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = ensure_home(home.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("another principal's pre-created home must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The missing leaf beneath another principal's directory: the
    // creation would land under its ancestors, so the trust check
    // refuses it the same way — and nothing is created.
    let missing = parent.join("bob");
    let error = home_usable(missing.to_str().unwrap())
        .expect_err("a missing home beneath another principal must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = ensure_home(missing.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("a missing home beneath another principal must never be created");
        assert!(error.contains("adopt"), "{error}");
    }
    assert!(!missing.exists(), "the refused home is never created");
    let meta = std::fs::metadata(&home).unwrap();
    assert_eq!(
        meta.uid(),
        nobody,
        "the refused directory is never handed to the account"
    );
    std::fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn an_operator_owned_home_under_a_shared_path_is_repaired() {
    // The root lane's adoption: the operator's own directory under
    // a shared path is repaired with the account's identity — the
    // ownership moves and the operator's contents stay.
    if super::operating_uid() != 0 {
        return;
    }
    let parent = scratch("operator-owned");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
    let repaired = ensure_home(home.to_str().unwrap(), 65534, 65534, false)
        .expect("the operator's home must be adopted");
    assert!(repaired, "the ownership moves to the account");
    let meta = std::fs::metadata(&home).unwrap();
    assert_eq!(
        (meta.uid(), meta.gid()),
        (65534, 65534),
        "the directory belongs to the account now"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"operator skeleton\n",
        "the operator's contents survive the adoption"
    );
    std::fs::remove_dir_all(&parent).unwrap();
}

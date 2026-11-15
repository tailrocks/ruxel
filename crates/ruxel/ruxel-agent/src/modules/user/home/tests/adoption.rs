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
fn a_created_home_takes_the_policy_mode() {
    // The `useradd -m` shape: the host makes the home with its own
    // creation mode — 0700 under a strict `HOME_MODE` — owned by the
    // account. The creation's reconciliation repairs the mode to the
    // policy's 0750: check mode reports the pending repair without
    // touching the directory, apply enforces it, and a converged home
    // is no change. The leaf is staged under the operator's own tree:
    // an operator-owned leaf counts only under trusted ancestors now.
    use std::os::unix::fs::PermissionsExt;
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-adopt-created-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    std::fs::set_permissions(&anchor, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = anchor.join("alice");
    let (uid, gid) = own_identity();
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let entry = || super::super::entries::PasswdEntry {
        uid,
        gid,
        ..Default::default()
    };
    let mode = || std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
    assert!(
        reconcile_created_home(
            "ruxel-created-alice",
            home.to_str().unwrap(),
            &entry(),
            true,
            &|_| panic!("check mode takes nothing back"),
        )
        .unwrap(),
        "check mode reports the pending mode repair"
    );
    assert_eq!(mode(), 0o700, "check mode changes nothing");
    assert!(
        reconcile_created_home(
            "ruxel-created-alice",
            home.to_str().unwrap(),
            &entry(),
            false,
            &|_| panic!("the mode repair must not take the account back"),
        )
        .unwrap(),
        "apply repairs the mode"
    );
    assert_eq!(mode(), 0o750, "the created home takes the policy mode");
    assert!(
        !reconcile_created_home(
            "ruxel-created-alice",
            home.to_str().unwrap(),
            &entry(),
            false,
            &|_| panic!("a converged home takes nothing back"),
        )
        .unwrap(),
        "a converged home is no change"
    );
    std::fs::remove_dir_all(&anchor).unwrap();
}

#[test]
fn an_operator_placed_home_is_adopted() {
    // The operator's own staging under trusted ancestors: the directory
    // the operator placed is adopted with its contents, and the
    // reconciliation repairs it with the account's final identity.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-adopt-operator-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    std::fs::set_permissions(&anchor, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = anchor.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
    let (uid, gid) = own_identity();
    // The preflight admits the operator's staging.
    home_usable(home.to_str().unwrap()).expect("the operator's home must be usable");
    // Check mode reports the repair where the ownership drifted.
    assert!(
        ensure_home(home.to_str().unwrap(), uid + 1, gid + 1, true)
            .unwrap()
            .0,
        "the drifted operator home reports its repair"
    );
    // Apply mode with matching ownership is no change — accepted,
    // nothing to do — and keeps the skeleton.
    assert!(
        !ensure_home(home.to_str().unwrap(), uid, gid, false)
            .unwrap()
            .0,
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
    std::fs::remove_dir_all(&anchor).unwrap();
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
fn an_attacker_owned_closed_leaf_is_refused() {
    // The finding's shape: another principal's 0700 leaf under trusted
    // ancestors — the mode-only fallback adopted it and handed the
    // account the principal's planted contents at login. Only the root
    // lane can stage another principal's ownership, so the unprivileged
    // lanes skip it.
    use std::os::unix::fs::PermissionsExt;
    if super::operating_uid() != 0 {
        return;
    }
    // A trusted tree straight under the root: root-owned and closed, so
    // the refusal below is the leaf's own, not its ancestors'.
    let parent = std::path::Path::new("/").join(format!("ruxel-adopt-leaf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    let stranger = 65534u32;
    rustix::fs::chown(
        &home,
        Some(rustix::fs::Uid::from_raw(stranger)),
        Some(rustix::fs::Gid::from_raw(stranger)),
    )
    .unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    // No passwd entry claims the scratch home, so no intended account
    // admits it: the preflight refuses before any mutation.
    assert_eq!(
        super::super::adoption::intended_claim(home.to_str().unwrap()),
        None,
        "the scratch home names no intended account"
    );
    let error = home_usable(home.to_str().unwrap())
        .expect_err("another principal's closed leaf must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = ensure_home(home.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("another principal's closed leaf must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The pinned adoption refuses the replacement the same way: the
    // descriptors name the leaf the walk opened.
    let (fd, _) = super::open_home(home.to_str().unwrap(), false).unwrap();
    let error = super::adoption_pinned(home.to_str().unwrap(), &fd)
        .expect_err("the pinned adoption must refuse another principal's leaf");
    assert!(error.contains("adopt"), "{error}");
    // Nothing was adopted: the ownership and the planted contents are
    // exactly what the principal left.
    let meta = std::fs::metadata(&home).unwrap();
    assert_eq!(
        meta.uid(),
        stranger,
        "the refused directory is never handed to the account"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"evil\n",
        "the planted contents are preserved, never handed over"
    );
    std::fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn the_passwd_home_names_its_intended_account() {
    // The lookup the leaf rule weighs: the account whose home field
    // names the path, name and number bound from the same line. Every
    // host lists an account, so the hit is portable — and a scratch
    // home misses, so the create flow's not-yet-existing account never
    // fails a valid creation. The home must be solely claimed — system
    // homes are shared in the wild, and a shared path names no one.
    let host = std::fs::read_to_string("/etc/passwd").unwrap();
    let (name, uid, claimed) = host
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let uid: u32 = fields.get(2)?.parse().ok()?;
            Some((fields.first()?.to_string(), uid, fields.get(5)?.to_string()))
        })
        .find(|(name, uid, home)| {
            super::super::adoption::intended_claim(home) == Some((name.clone(), *uid))
        })
        .expect("the host lists a solely claimed home");
    assert_eq!(
        super::super::adoption::intended_claim(&claimed),
        Some((name, uid)),
        "the passwd home names its account"
    );
    let missing = std::env::temp_dir().join(format!("ruxel-adopt-miss-{}", std::process::id()));
    assert_eq!(
        super::super::adoption::intended_claim(missing.to_str().unwrap()),
        None,
        "an unclaimed home names no intended account"
    );
}

#[test]
fn a_home_two_accounts_list_names_no_intended_account() {
    // The finding's shape, over canned passwd content: both the victim's
    // and the attacker's lines name the home. No first match may adopt
    // another account's leaf; the shared path names no one.
    let passwd =
        "victim:x:1001:1001::/home/alice:/bin/sh\nattacker:x:1002:1002::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::adoption::sole_claimant(passwd, "/home/alice"),
        None,
        "a shared home cannot be attributed to either owner"
    );
    // The reverse order shares it the same way: no first match admits
    // a leaf another account's line names.
    let passwd =
        "attacker:x:1002:1002::/home/alice:/bin/sh\nvictim:x:1001:1001::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::adoption::sole_claimant(passwd, "/home/alice"),
        None,
        "the order of the lines must not matter"
    );
    // A home one account lists still names it, name and number bound
    // from the same line; an unlisted home names no one.
    let passwd = "victim:x:1001:1001::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::adoption::sole_claimant(passwd, "/home/alice"),
        Some(("victim".to_string(), 1001)),
        "a solely claimed home names its account"
    );
    assert_eq!(
        super::super::adoption::sole_claimant(passwd, "/home/bob"),
        None,
        "an unlisted home names no account"
    );
}

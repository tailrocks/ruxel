use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// A scratch directory unique to the test.
fn scratch(scenario: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-adopt-{scenario}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The identity the test process owns, read off a probe file of it.
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

/// The repair and the preflight the tests drive: the account's number
/// is unknown, so no intended claim can match it.
fn repair(home: &str, uid: u64, gid: u64, check: bool) -> Result<bool, String> {
    ensure_home(
        home,
        uid,
        gid,
        check,
        &ManagedAccount::unnumbered("test"),
        &mut Vec::new(),
    )
}

fn usable(home: &str) -> Result<(), String> {
    home_usable(home, &ManagedAccount::unnumbered("test"))
}

#[test]
fn a_claim_by_another_account_never_names_the_managed_one() {
    // Managing Alice for Bob's claimed home: the name alone refuses.
    // Her own line names her only with her number on the leaf.
    let alice = ManagedAccount {
        name: "alice",
        uid: Some(1001),
    };
    let unnumbered = ManagedAccount::unnumbered("alice");
    assert!(!managed_claim("bob", 1002, 1002, &alice));
    assert!(managed_claim("alice", 1001, 1001, &alice));
    assert!(!managed_claim("alice", 9999, 9999, &alice));
    assert!(!managed_claim("alice", 1001, 1002, &alice));
    assert!(!managed_claim("alice", 1001, 1001, &unnumbered));
}

#[test]
fn an_attacker_writable_home_is_refused_never_adopted() {
    // A pre-created home under the shared-writable `/tmp`: a non-root
    // principal may have placed and filled it, so the adoption is
    // refused in every mode instead of owning it over.
    let parent = scratch("attack");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    let (uid, gid) = own_identity();
    let error =
        usable(home.to_str().unwrap()).expect_err("a writable pre-created home must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = repair(home.to_str().unwrap(), uid, gid, check_mode)
            .expect_err("a writable pre-created home must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The creation path refuses the same way and takes the account back.
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
    // Nothing was adopted: the ownership and the planted contents remain.
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
    // mode, and the creation's reconciliation repairs it to the
    // policy's 0750 — check mode reports, apply enforces, and a
    // converged home is no change.
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
    // The operator's own staging under trusted ancestors: adopted
    // with its contents, and reconciled with the final identity.
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
    usable(home.to_str().unwrap()).expect("the operator's home must be usable");
    assert!(
        repair(home.to_str().unwrap(), uid + 1, gid + 1, true).unwrap(),
        "the drifted operator home reports its repair"
    );
    assert!(
        !repair(home.to_str().unwrap(), uid, gid, false).unwrap(),
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
    // The declared home pre-created by another uid under a
    // shared-writable path. Only the root lane can stage another
    // uid's directory, so the unprivileged lanes skip it.
    if super::operating_uid() != 0 {
        return;
    }
    let parent = scratch("attack-owned");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    let nobody = 65534u32;
    rustix::fs::chown(
        &home,
        Some(rustix::fs::Uid::from_raw(nobody)),
        Some(rustix::fs::Gid::from_raw(nobody)),
    )
    .unwrap();
    let error = usable(home.to_str().unwrap())
        .expect_err("another principal's pre-created home must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = repair(home.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("another principal's pre-created home must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The missing leaf beneath another principal's directory: the
    // creation would land under it, so the trust check refuses it
    // the same way — and nothing is created.
    let missing = parent.join("bob");
    let error = usable(missing.to_str().unwrap())
        .expect_err("a missing home beneath another principal must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = repair(missing.to_str().unwrap(), 0, 0, check_mode)
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
    // Another principal's 0700 leaf under trusted ancestors. Only
    // the root lane can stage another principal's ownership, so the
    // unprivileged lanes skip it.
    use std::os::unix::fs::PermissionsExt;
    if super::operating_uid() != 0 {
        return;
    }
    // A trusted tree straight under the root, so the refusal below
    // is the leaf's own, not its ancestors'.
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
    // No passwd entry claims the scratch home.
    assert_eq!(
        super::super::adoption::intended_claim(home.to_str().unwrap()),
        None,
        "the scratch home names no intended account"
    );
    let error = usable(home.to_str().unwrap())
        .expect_err("another principal's closed leaf must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = repair(home.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("another principal's closed leaf must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
    // The pinned adoption refuses the replacement the same way.
    let fd = super::open_home(home.to_str().unwrap(), false, &mut Vec::new()).unwrap();
    let managed = ManagedAccount::unnumbered("test");
    let error = super::adoption_pinned(home.to_str().unwrap(), &fd, &managed)
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
    // names the path. The home must be solely claimed — a shared path
    // names no one.
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
    // Canned passwd content: both the victim's and the attacker's
    // lines name the home, and no first match may adopt another
    // account's leaf — the shared path names no one.
    let passwd =
        "victim:x:1001:1001::/home/alice:/bin/sh\nattacker:x:1002:1002::/home/alice:/bin/sh\n";
    assert_eq!(
        super::super::adoption::sole_claimant(passwd, "/home/alice"),
        None,
        "a shared home cannot be attributed to either owner"
    );
    // The reverse order shares it the same way.
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

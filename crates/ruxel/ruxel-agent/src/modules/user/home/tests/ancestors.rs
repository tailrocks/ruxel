/// The ancestor walk itself: a shared-writable ancestor refuses the
/// adoption whatever stands beneath it, and the operator's own tree
/// accepts it — over paths the walk only inspects, never changes.
#[test]
fn untrusted_ancestors_refuse_the_adoption() {
    // The shared directory itself, resolved past any symlink: it
    // is world-writable on every platform (1777 on Linux, 1777
    // through the `/tmp` symlink on macOS), so a home beneath it
    // may have been placed by any principal. The intermediate
    // directory exists, so the walk reaches the untrusted ancestor
    // instead of ending at a missing component.
    let shared = std::fs::canonicalize("/tmp")
        .expect("the shared directory resolves")
        .join(format!("ruxel-adopt-ancestors-{}", std::process::id()));
    std::fs::create_dir_all(&shared).unwrap();
    let error = super::trusted_ancestors(shared.join("alice").to_str().unwrap())
        .expect_err("a home under a shared-writable ancestor must be refused");
    assert!(error.contains("refusing to adopt"), "{error}");
    std::fs::remove_dir_all(&shared).unwrap();
    // The operator's own tree accepts it: a directory the process
    // owns, writable by no one else, under ancestors that are the
    // operator's or root's and writable by no one else.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-adopt-trust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    super::trusted_ancestors(anchor.join("alice").to_str().unwrap())
        .expect("the operator's own tree must accept the adoption");
    std::fs::remove_dir_all(&anchor).unwrap();
}

/// A missing home beneath an attacker-writable ancestor: the repair
/// would create the home there and then own it, so the parent could
/// rename the fresh directory aside and plant its own contents at the
/// passwd home path. The creation is refused in every mode before the
/// account is touched — fresh evidence after the existing-home adoption
/// fix, which left the missing leaf bypassing the ancestor trust check.
#[test]
fn a_missing_home_beneath_an_attacker_writable_ancestor_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    // A directory the test process owns but leaves writable to others:
    // writable-by-others is untrusted whatever the owner, so no
    // privilege is needed to stage the attacker's position.
    let parent = std::env::temp_dir().join(format!("ruxel-adopt-writable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = parent.join("alice");
    let error = super::home_usable(home.to_str().unwrap())
        .expect_err("a missing home beneath a writable ancestor must be refused");
    assert!(error.contains("refusing to adopt"), "{error}");
    for check_mode in [true, false] {
        let error = super::ensure_home(home.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("a missing home beneath a writable ancestor must never be created");
        assert!(error.contains("refusing to adopt"), "{error}");
    }
    assert!(!home.exists(), "the refused home is never created");
    std::fs::remove_dir_all(&parent).unwrap();
}

/// A missing home under the operator's own tree: the creation the
/// repair runs is admitted there, in check mode as much as in apply.
#[test]
fn a_missing_home_under_the_operators_own_tree_stays_usable() {
    use std::os::unix::fs::MetadataExt;
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-adopt-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    let home = anchor.join("alice");
    let probe = std::env::temp_dir().join(format!("ruxel-adopt-create-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    let meta = std::fs::metadata(&probe).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    std::fs::remove_file(&probe).unwrap();
    super::home_usable(home.to_str().unwrap())
        .expect("the operator's tree must accept the creation");
    assert!(
        super::ensure_home(home.to_str().unwrap(), uid, gid, true).unwrap(),
        "check mode reports the pending creation"
    );
    assert!(!home.exists(), "check mode creates nothing");
    assert!(
        super::ensure_home(home.to_str().unwrap(), uid, gid, false).unwrap(),
        "the missing home is created"
    );
    assert!(
        home.is_dir(),
        "the creation lands where the passwd path names"
    );
    std::fs::remove_dir_all(&anchor).unwrap();
}

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

/// Staging for the symlink-target findings: a root-owned tree holding
/// an admitted link whose target a lower-privileged principal
/// controls. Only the root lane can stage it — the admission rule
/// admits only a link a non-root principal cannot rewrite — so the
/// unprivileged lanes skip it.
fn admitted_link_farm(scenario: &str) -> Option<std::path::PathBuf> {
    if super::operating_uid() != 0 {
        return None;
    }
    let anchor =
        std::path::Path::new("/").join(format!("ruxel-target-{scenario}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir_all(&anchor).unwrap();
    Some(anchor)
}

/// The adoption of a home pre-created beneath an admitted link's
/// attacker-controlled target: the link stands where only the operator
/// could have placed it, but the target leaf is another principal's
/// with its login payload. The preflight and the repair refuse it in
/// every mode — fresh evidence beyond the existing-home fix, which
/// trusted only the link's own standing.
#[test]
fn a_home_planted_beneath_an_admitted_links_target_is_refused() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(anchor) = admitted_link_farm("attack") else {
        return;
    };
    // The attacker's farm: another principal's directory holding the
    // pre-created home with its payload, exactly the finding's shape.
    let evil = anchor.join("evil");
    std::fs::create_dir(&evil).unwrap();
    let home = evil.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".profile"), b"evil\n").unwrap();
    let stranger = 65534u32;
    for dir in [&evil, &home] {
        rustix::fs::chown(
            dir,
            Some(rustix::fs::Uid::from_raw(stranger)),
            Some(rustix::fs::Gid::from_raw(stranger)),
        )
        .unwrap();
    }
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    // The operator's admitted link in a root-owned unwritable
    // directory, pointing at the farm.
    let srv = anchor.join("srv");
    std::fs::create_dir(&srv).unwrap();
    std::os::unix::fs::symlink(&evil, srv.join("homes")).unwrap();
    let claimed = srv.join("homes/alice");
    let error = super::home_usable(claimed.to_str().unwrap())
        .expect_err("a home planted beneath an admitted link's target must be refused");
    assert!(error.contains("adopt"), "{error}");
    for check_mode in [true, false] {
        let error = super::ensure_home(claimed.to_str().unwrap(), 0, 0, check_mode)
            .expect_err("a planted home beneath an admitted link's target must never be adopted");
        assert!(error.contains("adopt"), "{error}");
    }
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
    std::fs::remove_dir_all(&anchor).unwrap();
}

/// The full target chain of nested admitted links: a trusted link may
/// point at a directory holding another trusted link, and that one at
/// a farm a non-root principal controls. The ancestor check resolves
/// and validates the complete chain — and still admits a chain that
/// stays on the operator's own directories.
#[test]
fn a_nested_admitted_link_chain_is_resolved_and_validated() {
    let Some(anchor) = admitted_link_farm("nested") else {
        return;
    };
    let stranger = 65534u32;
    let evil = anchor.join("evil");
    std::fs::create_dir(&evil).unwrap();
    rustix::fs::chown(
        &evil,
        Some(rustix::fs::Uid::from_raw(stranger)),
        Some(rustix::fs::Gid::from_raw(stranger)),
    )
    .unwrap();
    // The outer admitted link points at the operator's own middle
    // directory; the inner admitted link points at the farm.
    let outer = anchor.join("outer");
    std::fs::create_dir(&outer).unwrap();
    let middle = anchor.join("middle");
    std::fs::create_dir(&middle).unwrap();
    std::os::unix::fs::symlink(&middle, outer.join("mid")).unwrap();
    std::os::unix::fs::symlink(&evil, middle.join("deep")).unwrap();
    let error = super::trusted_ancestors(outer.join("mid/deep/alice").to_str().unwrap())
        .expect_err("a nested chain reaching an attacker farm must be refused");
    assert!(error.contains("adopt"), "{error}");
    // The chain that stays on the operator's own directories is
    // admitted: the middle directory holds no farm.
    std::fs::remove_file(middle.join("deep")).unwrap();
    super::trusted_ancestors(outer.join("mid/alice").to_str().unwrap())
        .expect("a nested chain over the operator's own directories must be admitted");
    std::fs::remove_dir_all(&anchor).unwrap();
}

/// A loop of admitted links ends the walk instead of the operation.
#[test]
fn a_symlink_loop_ends_the_walk_instead_of_the_operation() {
    let Some(anchor) = admitted_link_farm("loop") else {
        return;
    };
    std::os::unix::fs::symlink(anchor.join("second"), anchor.join("first")).unwrap();
    std::os::unix::fs::symlink(anchor.join("first"), anchor.join("second")).unwrap();
    let error = super::trusted_ancestors(anchor.join("first/alice").to_str().unwrap())
        .expect_err("a symlink loop must end the walk");
    assert!(error.contains("too many symlinks"), "{error}");
    std::fs::remove_dir_all(&anchor).unwrap();
}

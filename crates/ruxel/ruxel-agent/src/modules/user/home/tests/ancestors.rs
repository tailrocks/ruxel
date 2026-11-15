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

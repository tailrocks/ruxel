//! The mode a created home gets: the policy's, not the umask's.

use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// The environment variable that marks the process the case re-executes
/// itself in: the umask is the process's own, so a restrictive one
/// must not reach the tests that run beside it.
const OWN_UMASK: &str = "RUXEL_TEST_OWN_UMASK";

/// The case's own name in the test binary: the process of its own runs
/// exactly this one.
const CASE: &str =
    "modules::user::tests::home_modes::a_created_home_gets_the_policy_mode_under_any_umask";

#[test]
fn a_created_home_gets_the_policy_mode_under_any_umask() {
    if std::env::var_os(OWN_UMASK).is_none() {
        // The parent: the case runs in a process of its own, where the
        // restrictive umask is the only one. The umask is the
        // process's own — a change to it here would reach every test
        // that runs beside it.
        let exe = std::env::current_exe().unwrap();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "umask 0077; exec '{}' --exact {} --nocapture",
                exe.display(),
                CASE
            ))
            .env(OWN_UMASK, "1")
            .status()
            .expect("the case must run in a process of its own");
        assert!(
            status.success(),
            "the case must pass in the process of its own"
        );
        return;
    }
    // The child: the umask is 0077, so a mode that is only requested
    // at the creation comes out 0700 — the policy's mode must be
    // enforced on the directory itself, and on the parents it creates.
    let base = std::env::temp_dir().join(format!("ruxel-home-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let home = base.join("homes/alice");
    let meta = std::fs::metadata(&base).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    assert!(
        super::super::ensure_home(home.to_str().unwrap(), uid, gid, false).unwrap(),
        "the missing home is created"
    );
    let mode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o750,
        "the home's mode is the policy's, not the umask's"
    );
    let parent = std::fs::metadata(home.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        parent, 0o755,
        "the created parent is traversable and never world-writable"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

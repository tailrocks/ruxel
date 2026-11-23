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
    // The missing home is created beneath its ancestors, so the case
    // stages it under the operator's own tree: a shared-writable
    // ancestor would refuse the creation before any mutation.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-home-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir_all(&anchor).unwrap();
    let home = anchor.join("homes/alice");
    let meta = std::fs::metadata(&anchor).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    let managed = super::super::home::ManagedAccount::unnumbered("test");
    let mut added = Vec::new();
    assert!(
        super::super::ensure_home(
            home.to_str().unwrap(),
            uid,
            gid,
            false,
            &managed,
            &mut added
        )
        .unwrap(),
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
    std::fs::remove_dir_all(&anchor).unwrap();
}

#[test]
fn an_adopted_home_takes_the_policy_mode_with_the_ownership() {
    // The short-circuit's shape: the creation reconciles an
    // operator-staged 0755 home whose ownership drifts from the new
    // account's — the ownership moves and the mode must still take the
    // 0750 policy. The `||` used to skip the mode repair whenever the
    // ownership moved, adopting the home world-readable.
    let parent = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-adopt-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    // The staging anchor itself is trusted (0755): the adopted leaf is
    // what the adoption judges, and an untrusted ancestor would refuse
    // the adoption before any repair runs.
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = parent.join("alice");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(home.join(".profile"), b"operator skeleton\n").unwrap();
    let staged = std::fs::metadata(&home).unwrap();
    let (uid, gid) = (u64::from(staged.uid()), u64::from(staged.gid()));
    assert_eq!(
        staged.mode() & 0o777,
        0o755,
        "the staged home is world-readable"
    );
    // An ownership the test process may move without privilege: the
    // root lane hands the staged home to the nobody account, any other
    // lane drifts the group to one of its own.
    let (owner, group) = if uid == 0 {
        (65534u64, 65534u64)
    } else if let Some(drifted) = member_gid_other_than(gid) {
        (uid, drifted)
    } else {
        std::fs::remove_dir_all(&parent).unwrap();
        return;
    };
    let changed = super::super::reconcile_created_home(
        "ruxel-adopt-alice",
        home.to_str().unwrap(),
        &super::super::PasswdEntry {
            uid: owner,
            gid: group,
            ..Default::default()
        },
        false,
        &|_| panic!("the adoption must not take the account back"),
    )
    .expect("the operator's staged home must be adopted");
    assert!(changed, "the ownership repair is a change");
    let repaired = std::fs::metadata(&home).unwrap();
    assert_eq!(
        (u64::from(repaired.uid()), u64::from(repaired.gid())),
        (owner, group),
        "the adoption moves the ownership"
    );
    assert_eq!(
        repaired.mode() & 0o777,
        0o750,
        "the adopted home takes the policy mode with the ownership"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"operator skeleton\n",
        "the operator's contents survive the adoption"
    );
    std::fs::remove_dir_all(&parent).unwrap();
}

/// Another group the test process belongs to: a non-root lane cannot
/// hand a directory to another uid, but it may move the group among
/// its own — the ownership drift the adoption then repairs.
fn member_gid_other_than(gid: u64) -> Option<u64> {
    let output = std::process::Command::new("id").arg("-G").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.split_whitespace()
        .filter_map(|field| field.parse::<u64>().ok())
        .find(|candidate| *candidate != gid)
}

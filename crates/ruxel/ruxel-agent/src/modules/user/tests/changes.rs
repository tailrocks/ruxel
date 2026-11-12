//! Identity changes verify the live entry, not the declared name: the
//! live uid-0 account and gid-0 group are the host's to keep whatever
//! the workflow calls them.

use super::super::*;

#[test]
fn identity_changes_verify_the_live_entry_not_the_declared_name() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // root is this host's live uid-0 account: a workflow uid may not move
    // the superuser off uid 0, whatever the account is named.
    let error = run(&json!({"name":"root", "uid":1234}), &ctx)
        .expect_err("a uid change on the live uid-0 account must be refused");
    assert!(error.contains("uid 0"), "{error}");
    // The GID-0 group likewise, whatever it is named (root, wheel, …).
    let host = std::fs::read_to_string("/etc/group").unwrap();
    let name = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 3 && f[2] == "0").then(|| f[0].to_string())
        })
        .expect("a gid-0 group");
    let error = group(&json!({"name":name, "gid":1500}), &ctx)
        .expect_err("a gid change on the live gid-0 group must be refused");
    assert!(error.contains("gid 0"), "{error}");
    // A uid change of a non-superuser account is ordinary work, and the
    // guards pin the daemon scenario on canned entries.
    let app = PasswdEntry {
        uid: 1500,
        gid: 1500,
        ..PasswdEntry::default()
    };
    assert!(identity_change_allowed("app", &app).is_ok());
    let toor = PasswdEntry {
        uid: 0,
        ..PasswdEntry::default()
    };
    assert!(identity_change_allowed("toor", &toor).is_err());
    assert!(group_change_allowed("wheel", 0).is_err());
    assert!(group_change_allowed("docker", 999).is_ok());
}

#[test]
fn a_declared_home_directory_is_reconciled() {
    use std::os::unix::fs::MetadataExt;
    // The identity the new directory is created with: a probe file of the
    // test process, so no chown is needed unprivileged.
    let probe = std::env::temp_dir().join(format!("ruxel-home-probe-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    let meta = std::fs::metadata(&probe).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    std::fs::remove_file(&probe).unwrap();
    let dir = std::env::temp_dir().join(format!("ruxel-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // usermod -d moves the passwd field only: a missing directory is the
    // agent's to create, even when the home field already matches.
    assert!(super::super::ensure_home(dir.to_str().unwrap(), uid, gid, true).unwrap());
    assert!(!dir.exists(), "check mode creates nothing");
    assert!(super::super::ensure_home(dir.to_str().unwrap(), uid, gid, false).unwrap());
    assert!(dir.is_dir(), "the declared home exists");
    assert!(!super::super::ensure_home(dir.to_str().unwrap(), uid, gid, false).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

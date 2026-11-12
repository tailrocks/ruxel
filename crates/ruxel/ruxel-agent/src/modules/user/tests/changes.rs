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

#[test]
fn a_declared_shell_must_exist_and_be_executable() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // useradd and usermod accept a missing or non-executable shell and
    // only warn, leaving the account unable to log in while the apply
    // reports success.
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell":"/nonexistent/sh"}),
        &ctx,
    )
    .expect_err("a missing shell must be refused before the account is touched");
    assert!(error.contains("shell"), "{error}");
    let probe = std::env::temp_dir().join(format!("ruxel-shell-probe-{}", std::process::id()));
    std::fs::write(&probe, b"#!/bin/sh\n").unwrap();
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "shell":probe.to_str().unwrap()}),
        &ctx,
    )
    .expect_err("a non-executable shell must be refused");
    assert!(error.contains("shell"), "{error}");
    std::fs::remove_file(&probe).unwrap();
    assert!(
        run(
            &json!({"name":"ruxel-synthetic-user", "shell":"/bin/sh"}),
            &ctx
        )
        .is_ok()
    );
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
    // `usermod -d` moves the passwd field even when the declared home is
    // a regular file, so the path is validated before any mutation: a
    // refused apply must leave the account as it was.
    let error = run(
        &json!({"name":"ruxel-synthetic-user", "home":probe.to_str().unwrap()}),
        &ctx,
    )
    .expect_err("a home that is a regular file must be refused before the account changes");
    assert!(error.contains("home"), "{error}");
    // The home helper refuses the same path in both modes.
    assert!(super::super::home_usable(probe.to_str().unwrap()).is_err());
    std::fs::remove_file(&probe).unwrap();
}

#[test]
fn an_existing_home_directory_reports_drifted_ownership() {
    use std::os::unix::fs::MetadataExt;
    // The identity the test process owns: a probe file of it.
    let probe = std::env::temp_dir().join(format!("ruxel-home-drift-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    let meta = std::fs::metadata(&probe).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    std::fs::remove_file(&probe).unwrap();
    let dir = std::env::temp_dir().join(format!("ruxel-home-owned-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    // A home that exists but drifted to another identity is the agent's
    // to reconcile — the early return reported no change and left the
    // directory belonging to no one the account declares.
    assert!(
        super::super::ensure_home(dir.to_str().unwrap(), uid + 1, gid + 1, true).unwrap(),
        "check mode reports the pending repair"
    );
    assert!(
        !super::super::ensure_home(dir.to_str().unwrap(), uid, gid, false).unwrap(),
        "matching ownership is no change"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_created_home_takes_the_final_primary_gid() {
    // When one step changes the primary group, `usermod -g` applies
    // before the home is created: the directory must take the requested
    // group's gid, never the account's old one, which no later apply
    // would repair.
    let host = std::fs::read_to_string("/etc/group").unwrap();
    let (name, gid) = host
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() >= 3 && f[2] != "0" {
                f[2].parse::<u64>().ok().map(|gid| (f[0].to_string(), gid))
            } else {
                None
            }
        })
        .expect("a non-root group on the host");
    assert_eq!(super::super::home_gid(Some(&name), 1500).unwrap(), gid);
    assert_eq!(super::super::home_gid(None, 1500).unwrap(), 1500);
}

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

#[test]
fn a_created_account_creates_its_private_group_explicitly() {
    // The footprint model claims the same-name group of an account that
    // names no primary group: `useradd` must create it explicitly (`-U`),
    // never leaning on the host's USERGROUPS_ENAB setting, or the claim
    // is false on hosts configured the other way.
    assert_eq!(super::super::private_group_flag(None), Some("-U"));
    assert_eq!(super::super::private_group_flag(Some("docker")), None);
}

#[test]
fn a_declared_home_that_is_a_symlink_is_refused_never_chowned() {
    use std::os::unix::fs::MetadataExt;
    // A home in an attacker-writable parent that is a symlink to a
    // directory (`/tmp/alice-home -> /etc`) would be followed by the
    // metadata probe and by chown: the TARGET's ownership would
    // transfer to the managed account as root. A symlink is refused and
    // ownership changes never follow one.
    let target = std::env::temp_dir().join(format!("ruxel-home-target-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir(&target).unwrap();
    let link = std::env::temp_dir().join(format!("ruxel-home-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let meta = std::fs::metadata(&target).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    assert!(super::super::home_usable(link.to_str().unwrap()).is_err());
    assert!(super::super::ensure_home(link.to_str().unwrap(), uid, gid, true).is_err());
    assert!(super::super::ensure_home(link.to_str().unwrap(), uid, gid, false).is_err());
    let after = std::fs::metadata(&target).unwrap();
    assert_eq!(
        after.uid(),
        meta.uid(),
        "the symlink target must not be chowned"
    );
    assert_eq!(
        after.gid(),
        meta.gid(),
        "the symlink target must not be chowned"
    );
    std::fs::remove_file(&link).unwrap();
    std::fs::remove_dir_all(&target).unwrap();
}

#[test]
fn a_home_lookup_error_beyond_not_found_is_refused_before_the_account_changes() {
    // `/srv/file/alice` where `/srv/file` is a regular file: the lookup
    // fails `NotADirectory` on an ancestor — not a missing home. Taking
    // the catch-all for usable would let `usermod -d` move the passwd
    // field before `ensure_home` reports the filesystem error, leaving a
    // partially applied account after failure: only `NotFound` is
    // usable, and every other lookup error propagates before any
    // mutation.
    let dir = std::env::temp_dir().join(format!("ruxel-home-notdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("f");
    std::fs::write(&file, b"").unwrap();
    let nested = dir.join("f/alice");
    let error = super::super::home_usable(nested.to_str().unwrap())
        .expect_err("NotADirectory must not be usable");
    assert!(error.contains("home"), "{error}");
    // The same path through the operation is refused before the account
    // is touched, in both modes.
    for check_mode in [true, false] {
        let ctx = ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let error = run(
            &json!({"name":"ruxel-synthetic-user", "home":nested.to_str().unwrap()}),
            &ctx,
        )
        .expect_err("a home under a regular file must be refused before the account changes");
        assert!(error.contains("home"), "{error}");
    }
    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn the_creation_passes_the_claimed_home_explicitly() {
    // The lowering's footprint reserves `/home/<name>` where the step
    // names no home, and `useradd` must put the account there: the host's
    // configured base directory (`useradd -D`) is not the modeled one,
    // so the home is passed explicitly, declared or not.
    assert_eq!(super::super::claimed_home(None, "alice"), "/home/alice");
    assert_eq!(
        super::super::claimed_home(Some("/srv/alice"), "alice"),
        "/srv/alice"
    );
}

#[test]
fn an_existing_account_reconciles_its_claimed_private_group() {
    // A step that names no primary group claims the account's same-name
    // group — the footprint reserves it and the lowering's ordering
    // advertises it as provided — so an existing account creates it when
    // missing (`useradd -U` covers only the creation branch): a
    // dependent operation naming the group would otherwise fail to
    // resolve it.
    assert!(super::super::private_group_missing(None, false));
    assert!(!super::super::private_group_missing(None, true));
    assert!(!super::super::private_group_missing(Some("staff"), false));
}

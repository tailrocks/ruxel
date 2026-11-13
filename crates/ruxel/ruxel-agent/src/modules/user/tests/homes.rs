//! The home path the account's step manages before it touches the
//! account: every component of it, declared or claimed.

use super::super::*;

#[test]
fn a_home_under_a_symlinked_ancestor_is_refused_never_traversed() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let base = std::env::temp_dir().join(format!("ruxel-ancestor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let target = base.join("target");
    std::fs::create_dir_all(&target).unwrap();
    // The directory the link lives in is writable by everyone: the
    // attacker-writable parent a symlinked ancestor is planted in.
    let writable = base.join("writable");
    std::fs::create_dir(&writable).unwrap();
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o777)).unwrap();
    let link = writable.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let home = link.join("alice");
    // The walk must refuse the symlinked component before any lookup
    // follows it: `symlink_metadata` on the full path resolves the
    // ancestor, and `create_dir_all` and `lchown` are path-based — both
    // would build and chown the home beneath the link's target as root.
    let error = super::super::home_usable(home.to_str().unwrap())
        .expect_err("a home under a symlinked ancestor must be refused");
    assert!(error.contains("home"), "{error}");
    let meta = std::fs::metadata(&target).unwrap();
    let (uid, gid) = (u64::from(meta.uid()), u64::from(meta.gid()));
    let error = super::super::ensure_home(home.to_str().unwrap(), uid, gid, false)
        .expect_err("a home under a symlinked ancestor must never be created through the link");
    assert!(error.contains("home"), "{error}");
    assert!(
        !target.join("alice").exists(),
        "the home must not be created through the symlinked ancestor"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_claimed_default_home_is_refused_before_the_account_changes() {
    use std::os::unix::fs::MetadataExt;
    // The footprint's `/home/<name>` is the path the creation points
    // the new account at (`useradd -d`), declared or not: a regular
    // file there must be refused before the account is created, exactly
    // like a declared one — the account must not be left pointing at a
    // file, and `useradd -m` must not trip over it after the fact.
    let probe = std::env::temp_dir().join(format!("ruxel-claimed-home-{}", std::process::id()));
    std::fs::write(&probe, b"").unwrap();
    let expected = std::fs::metadata(&probe).unwrap();
    // `/home` is a real directory on Linux and a symlink into the data
    // volume on macOS, so `..` climbs to a different root on each: the
    // account name is the one whose claimed `/home/<name>` is the probe
    // file — identified by device and inode, the paths differ.
    let name = [
        format!("..{}", probe.display()),
        format!("../private{}", probe.display()),
    ]
    .into_iter()
    .find(|candidate| {
        std::fs::metadata(format!("/home/{candidate}"))
            .is_ok_and(|meta| meta.dev() == expected.dev() && meta.ino() == expected.ino())
    })
    .expect("a name whose claimed home is the probe file");
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let error = run(&json!({"name": name}), &ctx).expect_err(
        "a claimed default home that is a regular file must be refused before the account changes",
    );
    assert!(error.contains("home"), "{error}");
    std::fs::remove_file(&probe).unwrap();
}

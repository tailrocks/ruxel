//! The pre-existing home a failed reconciliation leaves behind: the
//! repair reads its ownership before it changes anything, and a
//! failure after the change restores it — while a refusal before any
//! change keeps its own error, never a take-back failure for a repair
//! that changed nothing.

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// A scratch directory unique to the test.
fn scratch(scenario: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-restore-{scenario}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ownership(path: &std::path::Path) -> (u64, u64) {
    let meta = std::fs::symlink_metadata(path).unwrap();
    (u64::from(meta.uid()), u64::from(meta.gid()))
}

/// The process's supplementary group ids, from the login database.
fn member_gids(primary: u64) -> Vec<u64> {
    let output = std::process::Command::new("id")
        .arg("-G")
        .output()
        .expect("id -G lists the process groups");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|gid| gid.parse::<u64>().ok())
        .filter(|gid| *gid != primary)
        .collect()
}

#[test]
fn prior_ownership_reads_the_directory_the_repair_found() {
    let dir = scratch("prior");
    let own = ownership(&dir);
    assert_eq!(
        prior_ownership(dir.to_str().unwrap()),
        Some(own),
        "a directory's ownership is its prior"
    );
    assert_eq!(
        prior_ownership(dir.join("absent").to_str().unwrap()),
        None,
        "a missing home has no prior to restore"
    );
    let file = dir.join("file");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        prior_ownership(file.to_str().unwrap()),
        None,
        "a non-directory is refused before any change, so it has no prior"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_restored_home_goes_back_to_the_ownership_the_repair_found() {
    // Thread 295: the repair changed the home's group and then failed.
    // The failure takes the home back to the ownership it found — the
    // operator's directory must not stay owned by the deleted
    // account's number. A host with no second group skips: the group
    // change is what makes the restoration observable.
    let dir = scratch("restore");
    let home = dir.join("home");
    std::fs::create_dir(&home).unwrap();
    let prior = ownership(&home);
    let Some(group) = member_gids(prior.1).first().copied() else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    // The change the repair made before it failed.
    std::os::unix::fs::chown(&home, None, Some(u32::try_from(group).unwrap())).unwrap();
    assert_eq!(ownership(&home), (prior.0, group));
    let error = restore_after_failure(
        home.to_str().unwrap(),
        Some(prior),
        &[],
        "synthetic failure".to_string(),
    );
    assert_eq!(
        error, "synthetic failure",
        "a clean restore keeps the error: {error}"
    );
    assert_eq!(
        ownership(&home),
        prior,
        "the home goes back to the ownership the repair found"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refusal_before_any_change_keeps_its_own_error() {
    // A refusal that changed nothing — the home still carries the
    // prior — must not append a take-back failure for a repair that
    // changed nothing.
    let dir = scratch("refusal");
    let home = dir.join("home");
    std::fs::create_dir(&home).unwrap();
    let prior = ownership(&home);
    let error = restore_after_failure(
        home.to_str().unwrap(),
        Some(prior),
        &[],
        "synthetic refusal".to_string(),
    );
    assert_eq!(
        error, "synthetic refusal",
        "an unchanged home keeps the refusal: {error}"
    );
    assert_eq!(ownership(&home), prior);
    // A home that is not there yet with nothing created keeps its
    // error the same way.
    let missing = dir.join("absent");
    let error = restore_after_failure(
        missing.to_str().unwrap(),
        None,
        &[],
        "synthetic refusal".to_string(),
    );
    assert_eq!(error, "synthetic refusal", "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_adoption_through_reconcile_keeps_its_error() {
    // A pre-existing home the adoption refuses before any change: a
    // 0777 scratch directory is no operator's own and no passwd entry
    // claims the scratch path, so the repair refuses with the
    // adoption error — and the take-back must not append to a refusal
    // that changed nothing.
    let dir = scratch("refused");
    let home = dir.join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o777)).unwrap();
    let (uid, gid) = ownership(&dir);
    let created = super::super::entries::PasswdEntry {
        uid,
        gid,
        ..Default::default()
    };
    let error = reconcile_created_home(
        "ruxel-restore-refused",
        home.to_str().unwrap(),
        &created,
        false,
        &|_| Ok(()),
    )
    .expect_err("another principal's home is refused");
    assert!(
        !error.contains(';'),
        "a refusal before any change keeps its own error: {error}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unrestorable_home_reports_the_cleanup_beside_the_error() {
    // A home that vanished after the repair moved it — the prior
    // names a directory that is no longer there — keeps the original
    // error and reports the cleanup beside it instead of silently
    // leaving the change behind.
    let dir = scratch("unrestorable");
    let home = dir.join("home");
    std::fs::create_dir(&home).unwrap();
    let prior = ownership(&home);
    std::fs::remove_dir(&home).unwrap();
    let error = restore_after_failure(
        home.to_str().unwrap(),
        Some(prior),
        &[],
        "synthetic failure".to_string(),
    );
    assert!(
        error.starts_with("synthetic failure; "),
        "the cleanup rides beside the error: {error}"
    );
    assert!(
        error.contains("user: own home"),
        "the report names the take-back: {error}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

use super::*;
use std::os::unix::fs::PermissionsExt;
#[test]
fn an_overwritten_entry_at_a_skel_name_is_kept_and_reported() {
    // Provenance is the template's own bytes at the name, not the name
    // alone: a concurrent overwrite of `.profile` — same pathname,
    // different bytes — is foreign data, and deleting it would destroy
    // evidence the preservation guarantee promises to keep.
    let scenario = format!("ruxel-created-overwrite-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&skel).unwrap();
    std::fs::write(skel.join(".profile"), b"skeleton\n").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".profile"), b"concurrent overwrite\n").unwrap();
    let error = rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect_err("an overwritten skel entry must not be deleted");
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert_eq!(
        std::fs::read(home.join(".profile")).unwrap(),
        b"concurrent overwrite\n",
        "the foreign bytes are kept and reported"
    );
    assert_eq!(
        std::fs::read(skel.join(".profile")).unwrap(),
        b"skeleton\n",
        "the template is untouched"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_symlink_at_a_skel_name_is_kept_and_reported() {
    // A planted link at a template name carries the template's pathname
    // but not its kind: removing it would delete foreign data — and
    // following it could delete through it — so the rollback refuses
    // without following either side.
    let scenario = format!("ruxel-created-symlink-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let target = std::env::temp_dir().join(format!("{scenario}-target"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&target);
    std::fs::create_dir_all(&skel).unwrap();
    std::fs::write(skel.join(".profile"), b"skeleton\n").unwrap();
    std::fs::write(&target, b"target\n").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(&target, home.join(".profile")).unwrap();
    let error = rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect_err("a symlink at a skel name must not be deleted");
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert!(
        std::fs::symlink_metadata(home.join(".profile"))
            .unwrap()
            .is_symlink(),
        "the planted link is kept and reported, never followed"
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"target\n",
        "nothing is deleted through the link"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&target);
}

#[test]
fn an_identical_skel_is_still_taken_back() {
    // The exact template — same kinds of entries with the same bytes,
    // the same link targets — is `useradd`'s own copy, so the rollback
    // still removes it: the provenance check must not strand homes the
    // old pathname check took back.
    let scenario = format!("ruxel-created-identical-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(skel.join("docs")).unwrap();
    std::fs::write(skel.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::write(skel.join("docs/readme"), b"skeleton\n").unwrap();
    std::os::unix::fs::symlink(".bashrc", skel.join(".link")).unwrap();
    std::fs::create_dir_all(home.join("docs")).unwrap();
    std::fs::write(home.join(".bashrc"), b"skeleton\n").unwrap();
    std::fs::write(home.join("docs/readme"), b"skeleton\n").unwrap();
    std::os::unix::fs::symlink(".bashrc", home.join(".link")).unwrap();
    rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect("the exact template copy must be taken back");
    assert!(!home.exists(), "the template-populated home is taken back");
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_missing_skel_keeps_everything_and_reports() {
    // No template means no provenance: any content is foreign, so the
    // home is kept and reported exactly as before.
    let scenario = format!("ruxel-created-noskel-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".profile"), b"skeleton\n").unwrap();
    let error = rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect_err("a missing template proves nothing and must refuse");
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert!(
        home.join(".profile").exists(),
        "nothing is deleted behind the refusal"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_unreadable_skel_keeps_everything_and_reports() {
    // A template the rollback cannot read proves nothing either: the
    // home is kept and reported rather than deleted on an unproven
    // pathname match.
    let scenario = format!("ruxel-created-unreadable-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&skel).unwrap();
    std::fs::write(skel.join(".profile"), b"skeleton\n").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".profile"), b"skeleton\n").unwrap();
    std::fs::set_permissions(&skel, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(skel.join(".profile")).is_ok() {
        std::fs::set_permissions(&skel, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&skel);
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let error = rollback_created_home_from(home.to_str().unwrap(), &skel)
        .expect_err("an unreadable template proves nothing and must refuse");
    std::fs::set_permissions(&skel, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert!(
        home.join(".profile").exists(),
        "nothing is deleted behind the refusal"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_plant_between_scan_and_removal_is_kept_and_reported() {
    // Thread 301: the scan proved the tree, then a concurrent writer
    // planted before the removal ran — the removal proves each entry
    // again just before removing it, so it stops at the plant instead
    // of deleting it, and the non-recursive final removal stops at
    // anything left behind rather than deleting it with the home.
    let scenario = format!("ruxel-created-race-{}", std::process::id());
    let skel = std::env::temp_dir().join(format!("{scenario}-skel"));
    let home = std::env::temp_dir().join(format!("{scenario}-home"));
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&skel).unwrap();
    std::fs::write(skel.join(".profile"), b"skeleton\n").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".profile"), b"skeleton\n").unwrap();
    // The scan the production rollback runs first: the tree proves.
    let scanned = relative_entries(&home).unwrap();
    assert!(!scanned.is_empty(), "the scan sees the template copy");
    // The concurrent writer plants after the scan and before the
    // removal — the shape a pre-removal `remove_dir_all` would delete
    // along with the home.
    std::fs::write(home.join("planted"), b"concurrent\n").unwrap();
    let error = remove_home_entries(home.to_str().unwrap(), &skel, &scanned)
        .expect_err("a plant after the scan must stop the removal");
    assert!(
        error.contains("take back created home"),
        "the refusal names the take-back: {error}"
    );
    assert_eq!(
        std::fs::read(home.join("planted")).unwrap(),
        b"concurrent\n",
        "the planted entry is kept and reported"
    );
    // The proven entries the removal already took back are gone —
    // they were the template's own — while the plant and the home
    // that holds it stay.
    assert!(
        !home.join(".profile").exists(),
        "the proven template entry is taken back"
    );
    assert!(
        home.exists(),
        "the home holding the plant is kept, not deleted with it"
    );
    let _ = std::fs::remove_dir_all(&skel);
    let _ = std::fs::remove_dir_all(&home);
}

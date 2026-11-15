//! The non-mutating rehearsal: the live path is never renamed, so no
//! interruption and no refusal can strand rewritten bytes at it — and
//! the rehearsal still stages the exact rewritten bytes and renames a
//! scratch name over a scratch name, so quota, directory-permission
//! and rename refusals still refuse before `sysctl -w` changes the
//! live value.

use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-rehearse-{name}-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    dir
}

fn live_bytes(dir: &std::path::Path) -> (String, Vec<u8>) {
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
        .expect("operation succeeds");
    (
        path.to_str().expect("path is text").to_owned(),
        b"net.test=2\n".to_vec(),
    )
}

fn dir_names(dir: &std::path::Path) -> Vec<String> {
    let mut left: Vec<String> = std::fs::read_dir(dir)
        .expect("operation succeeds")
        .map(|entry| {
            entry
                .expect("operation succeeds")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    left.sort();
    left
}

#[test]
fn the_rehearsal_leaves_the_live_file_untouched_and_leaves_no_scratch() {
    // The rehearsal stages the rewritten bytes under scratch names and
    // renames one over the other: the live path is never renamed, so
    // the file the rehearsal proves keeps its inode, its bytes, its
    // mode — and no scratch file is left behind.
    let dir = scratch("ok");
    let (text, before_bytes) = live_bytes(&dir);
    let path = std::path::Path::new(&text);
    let before = std::fs::metadata(path).expect("operation succeeds");
    super::super::preflight_persistence(&text, b"net.test=2\n", true)
        .expect("a replaceable target must pass the rehearsal");
    let after = std::fs::metadata(path).expect("operation succeeds");
    assert_eq!(
        (before.dev(), before.ino()),
        (after.dev(), after.ino()),
        "the rehearsal must never move the live file"
    );
    assert_eq!(
        after.mode() & 0o7777,
        0o640,
        "the rehearsal must preserve the live mode"
    );
    assert_eq!(
        std::fs::read(path).expect("operation succeeds"),
        before_bytes,
        "the rehearsal must preserve the live bytes"
    );
    assert_eq!(
        dir_names(&dir),
        vec!["x.conf".to_string()],
        "the rehearsal must leave no scratch behind"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn the_rehearsal_needs_no_link_aside_so_it_never_renames_the_live_path() {
    // Fault injection for the kill-between-renames window: the retired
    // rehearsal linked the live file aside under these names and
    // renamed the staged bytes over the live path, so a directory
    // squatting on them made its first step fail and the whole
    // rehearsal refuse. The non-mutating rehearsal stages and renames
    // under its own scratch names, so the squat changes nothing: the
    // preflight still passes with the live file untouched, which the
    // retired flow could not do — proving no step renames the live
    // path anymore.
    let dir = scratch("no-aside");
    let (text, _) = live_bytes(&dir);
    let path = std::path::Path::new(&text);
    for retired in [
        format!(".ruxel-sysctl-rehearse-{}", std::process::id()),
        format!(".ruxel-sysctl-staged-{}", std::process::id()),
    ] {
        std::fs::create_dir_all(dir.join(&retired)).expect("operation succeeds");
    }
    let before = std::fs::metadata(path).expect("operation succeeds");
    super::super::preflight_persistence(&text, b"net.test=1\n", true)
        .expect("a squat on the retired aside names must not refuse the rehearsal");
    let after = std::fs::metadata(path).expect("operation succeeds");
    assert_eq!(
        (before.dev(), before.ino()),
        (after.dev(), after.ino()),
        "the rehearsal must never move the live file"
    );
    assert_eq!(
        std::fs::read(path).expect("operation succeeds"),
        b"net.test=2\n",
        "the rehearsal must preserve the live bytes"
    );
    for retired in [
        format!(".ruxel-sysctl-rehearse-{}", std::process::id()),
        format!(".ruxel-sysctl-staged-{}", std::process::id()),
    ] {
        assert!(
            dir.join(&retired).is_dir(),
            "the rehearsal must not touch the retired names: {retired}"
        );
    }
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn a_blocked_scratch_stage_leaves_the_live_file_where_it_was() {
    // The staging is the rehearsal's first step: where it cannot be
    // made — here a directory squats on the first scratch name — the
    // rename never runs, and the live file keeps its inode and its
    // bytes, with no staged copy left behind.
    let dir = scratch("blocked-stage");
    let (text, _) = live_bytes(&dir);
    let path = std::path::Path::new(&text);
    let first = dir.join(format!(".ruxel-sysctl-rehearse-a-{}", std::process::id()));
    std::fs::create_dir_all(&first).expect("operation succeeds");
    let before = std::fs::metadata(path).expect("operation succeeds");
    let error = super::super::preflight_persistence(&text, b"net.test=1\n", true)
        .expect_err("the rehearsal must refuse");
    assert!(error.contains("cannot persist"), "{error}");
    let after = std::fs::metadata(path).expect("operation succeeds");
    assert_eq!(
        (before.dev(), before.ino()),
        (after.dev(), after.ino()),
        "a refused rehearsal must leave the live inode where it was"
    );
    assert_eq!(
        std::fs::read(path).expect("operation succeeds"),
        b"net.test=2\n",
        "a refused rehearsal must leave the live bytes alone"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn a_blocked_scratch_rename_leaves_the_live_file_where_it_was() {
    // The rename over the second scratch name is the operation under
    // test: where it cannot run — here a directory squats on the
    // rename target — the rehearsal refuses although the bytes staged
    // fine, and the live file keeps its inode and its bytes, with the
    // staged copy cleaned up behind the refusal.
    let dir = scratch("blocked-rename");
    let (text, _) = live_bytes(&dir);
    let path = std::path::Path::new(&text);
    let second = dir.join(format!(".ruxel-sysctl-rehearse-b-{}", std::process::id()));
    std::fs::create_dir_all(&second).expect("operation succeeds");
    let before = std::fs::metadata(path).expect("operation succeeds");
    let error = super::super::preflight_persistence(&text, b"net.test=1\n", true)
        .expect_err("the rehearsal must refuse");
    assert!(error.contains("cannot persist"), "{error}");
    let after = std::fs::metadata(path).expect("operation succeeds");
    assert_eq!(
        (before.dev(), before.ino()),
        (after.dev(), after.ino()),
        "a refused rehearsal must leave the live inode where it was"
    );
    assert_eq!(
        std::fs::read(path).expect("operation succeeds"),
        b"net.test=2\n",
        "a refused rehearsal must leave the live bytes alone"
    );
    assert!(
        !dir.join(format!(".ruxel-sysctl-rehearse-a-{}", std::process::id()))
            .exists(),
        "a refused rehearsal must leave no staged copy behind"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn the_rehearsal_stages_the_exact_rewritten_bytes() {
    // The rehearsal stages the rewritten bytes at their rewritten
    // size: a quota that fits the old content but not the new one must
    // refuse here, before `sysctl -w` changes the live value with no
    // boot entry behind it. The staging carries the new bytes — here
    // many times the live file's — and leaves the live file and no
    // scratch behind.
    let dir = scratch("exact");
    let (text, _) = live_bytes(&dir);
    let path = std::path::Path::new(&text);
    let mut rewritten = b"net.test=2\n".to_vec();
    rewritten.extend(std::iter::repeat_n(b'#', 65_537));
    rewritten.push(b'\n');
    super::super::preflight_persistence(&text, &rewritten, true)
        .expect("a replaceable target must stage the rewritten bytes");
    assert_eq!(
        std::fs::read(path).expect("operation succeeds"),
        b"net.test=2\n",
        "the rehearsal must leave the live bytes alone"
    );
    assert_eq!(
        dir_names(&dir),
        vec!["x.conf".to_string()],
        "the rehearsal must leave no scratch behind"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn a_missing_target_stages_the_exact_bytes_it_will_publish() {
    // The target is absent, so there is no live file to rehearse
    // against: the probe itself carries the exact bytes the publish
    // will write, and a quota that fits an empty file but not the new
    // content refuses here, before `sysctl -w`. Nothing is left behind
    // either way.
    let dir = scratch("missing");
    let path = dir.join("x.conf");
    let text = path.to_str().expect("path is text").to_owned();
    let rewritten = b"net.test=1\n# padding\n".to_vec();
    super::super::preflight_persistence(&text, &rewritten, true)
        .expect("a writable directory must stage the new file's bytes");
    assert!(
        !path.exists(),
        "the rehearsal must not publish the missing target"
    );
    assert!(
        std::fs::read_dir(&dir)
            .expect("operation succeeds")
            .next()
            .is_none(),
        "the rehearsal must leave no probe behind"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

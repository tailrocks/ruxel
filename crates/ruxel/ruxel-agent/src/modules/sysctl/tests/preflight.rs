//! The rehearsal the publishability probe runs: the live file is
//! never renamed away, so no interruption and no refusal strands the
//! configured path — and the rehearsal stages the exact rewritten
//! bytes, so a quota that fits the old content but not the new one
//! refuses before `sysctl -w` changes the live value.

use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn the_rehearsal_preserves_the_live_file_and_leaves_no_scratch() {
    // The rehearsal publishes a scratch copy carrying the rewritten
    // bytes over the live path and links the live inode back: the
    // configured path is never renamed away, so the file the rehearsal
    // proves keeps its inode, its bytes, its mode — and no scratch
    // file is left behind.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-rehearse-ok-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
        .expect("operation succeeds");
    let text = path.to_str().expect("path is text").to_owned();
    let before = std::fs::metadata(&path).expect("operation succeeds");
    super::super::preflight_persistence(&text, b"net.test=2\n", true)
        .expect("a replaceable target must pass the rehearsal");
    let after = std::fs::metadata(&path).expect("operation succeeds");
    assert_eq!(
        before.ino(),
        after.ino(),
        "the rehearsal must link the live inode back"
    );
    assert_eq!(
        after.mode() & 0o7777,
        0o640,
        "the rehearsal must preserve the live mode"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("operation succeeds"),
        "net.test=2\n",
        "the rehearsal must preserve the live bytes"
    );
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .expect("operation succeeds")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["x.conf".to_string()],
        "the rehearsal must leave no scratch behind: {left:?}"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn a_refused_rehearsal_leaves_the_live_file_where_it_was() {
    // The link aside is the rehearsal's first step: where it cannot
    // be made — here a directory squats on the scratch name — the
    // publish over the live path never runs, and the live file
    // keeps its inode and its bytes behind no missing path, with no
    // staged copy left behind.
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-rehearse-refused-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
    let aside = dir.join(format!(".ruxel-sysctl-rehearse-{}", std::process::id()));
    std::fs::create_dir_all(&aside).expect("operation succeeds");
    let staged = dir.join(format!(".ruxel-sysctl-staged-{}", std::process::id()));
    let text = path.to_str().expect("path is text").to_owned();
    let before = std::fs::metadata(&path).expect("operation succeeds");
    let error = super::super::preflight_persistence(&text, b"net.test=1\n", true)
        .expect_err("the rehearsal must refuse");
    assert!(error.contains("cannot persist"), "{error}");
    let after = std::fs::metadata(&path).expect("operation succeeds");
    assert_eq!(
        before.ino(),
        after.ino(),
        "a refused rehearsal must leave the live inode where it was"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("operation succeeds"),
        "net.test=2\n",
        "a refused rehearsal must leave the live bytes alone"
    );
    assert!(
        !staged.exists(),
        "a refused rehearsal must leave no staged copy behind"
    );
    std::fs::remove_dir_all(&dir).expect("operation succeeds");
}

#[test]
fn the_rehearsal_stages_the_exact_rewritten_bytes() {
    // The rehearsal is the publish's own operation at the rewritten
    // size: a quota that fits the old content but not the new one must
    // refuse here, before `sysctl -w` changes the live value with no
    // boot entry behind it. The staging carries the new bytes — here
    // many times the live file's — and leaves the live file and no
    // scratch behind.
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-rehearse-exact-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
    let text = path.to_str().expect("path is text").to_owned();
    let mut rewritten = b"net.test=2\n".to_vec();
    rewritten.extend(std::iter::repeat_n(b'#', 65_537));
    rewritten.push(b'\n');
    super::super::preflight_persistence(&text, &rewritten, true)
        .expect("a replaceable target must stage the rewritten bytes");
    assert_eq!(
        std::fs::read(&path).expect("operation succeeds"),
        b"net.test=2\n",
        "the rehearsal must leave the live bytes alone"
    );
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .expect("operation succeeds")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["x.conf".to_string()],
        "the rehearsal must leave no scratch behind: {left:?}"
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
    let dir = std::env::temp_dir().join(format!(
        "ruxel-sysctl-rehearse-missing-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
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

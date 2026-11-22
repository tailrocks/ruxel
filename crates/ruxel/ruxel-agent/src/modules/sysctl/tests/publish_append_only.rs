//! The publish probe on append-only targets and the scratch-name rehearsal.

use std::os::unix::fs::MetadataExt;

use serde_json::json;

/// Makes `path` append-only and whether this host could.
fn lock_target_append(path: &std::path::Path) -> bool {
    let (program, argument) = if cfg!(target_os = "linux") {
        ("chattr", "+a")
    } else {
        ("chflags", "uappend")
    };
    std::process::Command::new(program)
        .arg(argument)
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// Puts the file back into the state `lock_target_append` took it out of.
fn unlock_target_append(path: &std::path::Path) {
    let (program, argument) = if cfg!(target_os = "linux") {
        ("chattr", "-a")
    } else {
        ("chflags", "nouappend")
    };
    let _ = std::process::Command::new(program)
        .arg(argument)
        .arg(path)
        .status();
}

#[test]
fn an_append_only_target_is_refused_before_the_live_write() {
    // An append-only file sits in a writable directory and accepts
    // sibling writes, but the publish's rename over it fails — and a
    // write-open probe would pass it, so only the flag probe refuses
    // it before `sysctl -w` changes the live value with no persistent
    // entry behind it. The refusal names the persistence file and the
    // file itself is left exactly as it was.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-append-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").unwrap();
    if !lock_target_append(&path) {
        // This host cannot make a file append-only without a
        // privilege the test has not: the ordinary path above is
        // covered by the other tests' publishes.
        unlock_target_append(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    }
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let error = super::super::run(
        &json!({"name": "net.test", "value": "1", "sysctl_file": path, "reload": false}),
        &ctx,
        &super::super::SysctlManaged::default(),
    )
    .expect_err("an append-only target must be refused");
    assert!(error.contains("cannot persist"), "{error}");
    assert!(
        error.contains("locked against replacement"),
        "the read-only flag probe must refuse the locked file, not the rename: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "net.test=2\n",
        "a refused publish must leave the file untouched"
    );
    unlock_target_append(&path);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_publish_rehearsal_leaves_the_target_where_it_was() {
    // The preflight proves the publish under scratch names in the same
    // directory — the live path is never renamed — so the target's own
    // inode must come back untouched: the host's file is not moved, its
    // content survives, and no probe file is left behind.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-rehearse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.conf");
    std::fs::write(&path, "net.test=2\n").unwrap();
    let before = std::fs::metadata(&path).unwrap();
    super::super::preflight_persistence(path.to_str().unwrap(), b"net.test=2\n", true)
        .expect("a replaceable target must pass the rehearsal");
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(
        before.ino(),
        after.ino(),
        "the rehearsal must put the target's own inode back"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "net.test=2\n",
        "the rehearsal must leave the target's content alone"
    );
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["x.conf".to_string()],
        "the rehearsal must leave no probe file behind: {left:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

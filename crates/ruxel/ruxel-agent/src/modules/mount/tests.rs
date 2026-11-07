const FAKE_MOUNT: &str = r#"set -eu
kind=$1
shift
if [ "$kind" = umount ]; then
  [ "$1" = -- ] || exit 2
  if [ "$MOUNT_MODE" = fail_unmount ]; then echo synthetic-unmount-failure >&2; exit 1; fi
  printf '' > "$MOUNT_STATE"
  exit 0
fi
source=$6
if [ "$source" = /new ] && [ "$MOUNT_MODE" != ok ]; then
  echo synthetic-new-mount-failure >&2
  exit 1
fi
if [ "$source" = /old ] && [ "$MOUNT_MODE" = fail_both ]; then
  echo synthetic-restore-failure >&2
  exit 1
fi
printf '%s' "$source" > "$MOUNT_STATE"
"#;

use serde_json::json;

#[test]
fn rejects_newline_in_fstab_fields_before_io() {
    let ctx = super::ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    for (field, value) in [
        ("path", "/mnt\nbad"),
        ("src", "/dev/x\rbad"),
        ("fstype", "xfs\nbad"),
        ("opts", "defaults\nbad"),
    ] {
        let mut params =
            json!({"path": "/mnt/x", "src": "/dev/x", "fstype": "xfs", "opts": "defaults"});
        params[field] = json!(value);
        assert!(super::run(&params, &ctx).is_err(), "field {field}");
    }
}

#[test]
fn identifies_mount_drift() {
    use super::{MountAction, find_mount, mount_action};
    let mounts = "/dev/a /srv xfs rw,noatime 0 0\n";
    let current = find_mount(mounts, "/srv");
    assert_eq!(
        mount_action(current.as_ref(), "/dev/a", "xfs", "noatime,rw"),
        MountAction::None
    );
    assert_eq!(
        mount_action(current.as_ref(), "/dev/a", "xfs", "ro"),
        MountAction::Remount
    );
    assert_eq!(
        mount_action(current.as_ref(), "/dev/a", "xfs", "defaults"),
        MountAction::None
    );
    for option in ["ro", "nosuid", "nodev", "noexec", "sync"] {
        let mounts = format!("/dev/a /srv xfs rw,{option} 0 0\n");
        assert_eq!(
            mount_action(
                find_mount(&mounts, "/srv").as_ref(),
                "/dev/a",
                "xfs",
                "defaults"
            ),
            MountAction::Remount
        );
    }
    let restricted = find_mount("/dev/a /srv xfs rw,noexec 0 0\n", "/srv");
    assert_eq!(
        mount_action(restricted.as_ref(), "/dev/a", "xfs", "defaults,noexec"),
        MountAction::None
    );
    assert_eq!(
        mount_action(restricted.as_ref(), "/dev/a", "xfs", "noexec,defaults"),
        MountAction::None
    );
    assert_eq!(
        mount_action(current.as_ref(), "/dev/b", "xfs", "rw"),
        MountAction::Replace
    );
    assert_eq!(
        mount_action(None, "/dev/a", "xfs", "rw"),
        MountAction::Mount
    );
}

#[test]
fn normalizes_options_and_rewrites_matching_mountpoint() {
    assert!(super::opts_eq("rw,noatime", "noatime,rw"));
    let current = "# keep\n/dev/old /srv ext4 defaults 0 0\n";
    let (next, changed) = super::rewrite_fstab(current, "/dev/new", "/srv", "xfs", "rw");
    assert!(changed);
    assert_eq!(next, "# keep\n/dev/new\t/srv\txfs\trw\t0\t0\n");
    assert_eq!(
        super::rewrite_fstab(&next, "/dev/new", "/srv", "xfs", "rw"),
        (next, false)
    );
}

#[test]
fn fstab_encodes_newlines_for_roundtrip() {
    let (next, changed) = super::rewrite_fstab("", "/dev/a\nb", "/mnt/a\nb", "ext4", "rw");
    assert!(changed);
    assert_eq!(next, "/dev/a\\012b\t/mnt/a\\012b\text4\trw\t0\t0\n");
    assert_eq!(
        super::rewrite_fstab(&next, "/dev/a\nb", "/mnt/a\nb", "ext4", "rw"),
        (next, false)
    );
}

#[test]
fn rollback_unmounts_protect_option_like_paths() {
    let mount = super::Mounted {
        source: "/dev/a".into(),
        fstype: "ext4".into(),
        options: "rw".into(),
    };
    for action in [super::MountAction::Mount, super::MountAction::Replace] {
        let mut calls = Vec::new();
        let mut command = |program: &str, args: &[&str]| {
            calls.push((
                program.to_owned(),
                args.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ));
            Ok(())
        };
        let mut persist = || Err("synthetic failure".into());
        assert!(
            super::apply_mount(
                "-target",
                Some(&mount),
                &mount,
                action,
                &mut command,
                &mut persist
            )
            .is_err()
        );
        let unmounts: Vec<_> = calls
            .iter()
            .filter(|(program, _)| program == "umount")
            .collect();
        assert_eq!(
            unmounts.len(),
            if action == super::MountAction::Replace {
                2
            } else {
                1
            }
        );
        assert!(unmounts.iter().all(|(_, args)| args == &["--", "-target"]));
    }
}

#[test]
fn replacement_failure_restores_old_mount_and_preserves_fstab() {
    exercise_replace("fail_new", false, true, false);
}

#[test]
fn unmount_failure_preserves_old_mount_and_fstab() {
    exercise_replace("fail_unmount", false, true, false);
}

#[test]
fn fstab_failure_restores_old_mount() {
    exercise_replace("ok", true, true, false);
}

#[test]
fn replacement_success_persists_new_mount() {
    exercise_replace("ok", false, false, false);
}

#[test]
fn failed_restore_is_reported() {
    exercise_replace("fail_both", false, true, true);
}

fn exercise_replace(mode: &str, fail_persist: bool, expect_error: bool, fail_restore: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "ruxel-mount-replace-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let state = root.join("state");
    let fstab = root.join("fstab");
    let script = root.join("fake-mount.sh");
    std::fs::write(&state, "/old").unwrap();
    std::fs::write(&fstab, "old fstab\n").unwrap();
    std::fs::write(&script, FAKE_MOUNT).unwrap();
    let old = super::Mounted {
        source: "/old".into(),
        fstype: "ext4".into(),
        options: "rw".into(),
    };
    let new = super::Mounted {
        source: "/new".into(),
        fstype: "xfs".into(),
        options: "rw".into(),
    };
    let mut command = |program: &str, args: &[&str]| {
        let mut process = std::process::Command::new("sh");
        process
            .arg(&script)
            .arg(program)
            .args(args)
            .env("MOUNT_STATE", &state)
            .env("MOUNT_MODE", mode);
        super::super::run_checked(process).map(|_| ())
    };
    let mut persist = || {
        if fail_persist {
            return Err("synthetic fstab failure".into());
        }
        std::fs::write(&fstab, "new fstab\n").map_err(|e| e.to_string())
    };
    let result = super::replace_mount("/mount", &old, &new, &mut command, &mut persist);
    assert_eq!(result.is_err(), expect_error);
    if fail_restore {
        assert!(result.unwrap_err().contains("restoration failed"));
    }
    assert_eq!(
        std::fs::read_to_string(&fstab).unwrap(),
        if expect_error {
            "old fstab\n"
        } else {
            "new fstab\n"
        }
    );
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        if fail_restore {
            ""
        } else if expect_error {
            "/old"
        } else {
            "/new"
        }
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn refuses_unrestorable_mounts_before_replacement() {
    let device = super::Mounted {
        source: "/dev/fixture".into(),
        fstype: "xfs".into(),
        options: "rw".into(),
    };
    let info = "1 0 8:1 / /target rw - xfs /dev/fixture rw\n";
    assert!(super::ensure_replaceable(&device).is_ok());
    assert!(super::ensure_mount_root("/target", info).is_ok());
    let subtree = "1 0 8:1 /subdir /target rw - xfs /dev/fixture rw\n";
    assert!(super::ensure_mount_root("/target", subtree).is_err());
    assert!(super::ensure_mount_root("/missing", info).is_err());
    let tmpfs = super::Mounted {
        source: "tmpfs".into(),
        fstype: "tmpfs".into(),
        options: "rw".into(),
    };
    assert!(super::ensure_replaceable(&tmpfs).is_err());
}

#[test]
fn failed_initial_mount_and_remount_preserve_fstab() {
    for action in [super::MountAction::Mount, super::MountAction::Remount] {
        let old = super::Mounted {
            source: "/dev/old".into(),
            fstype: "ext4".into(),
            options: "rw".into(),
        };
        let new = super::Mounted {
            source: "/dev/new".into(),
            fstype: "xfs".into(),
            options: "ro".into(),
        };
        let mut calls = 0;
        let mut persist_called = false;
        let mut command = |_: &str, _: &[&str]| {
            calls += 1;
            let mut child = std::process::Command::new("sh");
            child.args(["-c", if calls == 1 { "exit 1" } else { "exit 0" }]);
            super::super::run_checked(child).map(|_| ())
        };
        let mut persist = || {
            persist_called = true;
            Ok(())
        };
        assert!(
            super::apply_mount(
                "/target",
                Some(&old),
                &new,
                action,
                &mut command,
                &mut persist
            )
            .is_err()
        );
        assert!(!persist_called);
    }
}

#[test]
fn failed_fstab_write_removes_new_mount_or_restores_options() {
    for action in [super::MountAction::Mount, super::MountAction::Remount] {
        let old = super::Mounted {
            source: "/dev/old".into(),
            fstype: "ext4".into(),
            options: "rw".into(),
        };
        let new = super::Mounted {
            source: "/dev/new".into(),
            fstype: "xfs".into(),
            options: "ro".into(),
        };
        let mut calls = Vec::new();
        let mut command = |program: &str, args: &[&str]| {
            calls.push((
                program.to_string(),
                args.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ));
            let mut child = std::process::Command::new("sh");
            child.args(["-c", "exit 0"]);
            super::super::run_checked(child).map(|_| ())
        };
        let mut persist = || Err("synthetic fstab failure".into());
        assert!(
            super::apply_mount(
                "/target",
                Some(&old),
                &new,
                action,
                &mut command,
                &mut persist
            )
            .is_err()
        );
        assert_eq!(calls.len(), 2);
        assert!(calls[1].0 == "umount" || calls[1].1.contains(&"remount,rw".to_string()));
    }
}

#[test]
fn fstab_escapes_whitespace_and_converges() {
    let (next, changed) = super::rewrite_fstab("", "/dev/a b", "/target with space", "ext4", "rw");
    assert!(changed);
    assert_eq!(
        next,
        "/dev/a\\040b\t/target\\040with\\040space\text4\trw\t0\t0\n"
    );
    assert_eq!(
        super::rewrite_fstab(&next, "/dev/a b", "/target with space", "ext4", "rw"),
        (next, false)
    );
}

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

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

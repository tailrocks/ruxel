#[test]
fn auto_preserves_source_and_options_checks() {
    use super::super::{MountAction, find_mount, mount_action};
    for fstype in ["ext4", "xfs"] {
        let mounts = format!("/synthetic/source /target {fstype} rw,noatime 0 0\n");
        let current = find_mount(&mounts, "/target");
        for (source, desired_type, options, action) in [
            ("/synthetic/source", "auto", "rw,noatime", MountAction::None),
            ("/synthetic/source", "auto", "ro", MountAction::Remount),
            ("/synthetic/other", "auto", "rw", MountAction::Replace),
            ("/synthetic/source", "other", "rw", MountAction::Replace),
        ] {
            assert_eq!(
                mount_action(current.as_ref(), source, desired_type, options),
                action
            );
        }
    }
    assert_eq!(
        mount_action(None, "/synthetic/source", "auto", "rw"),
        MountAction::Mount
    );
    let current = find_mount("/synthetic/source /target auto rw 0 0\n", "/target");
    assert_eq!(
        mount_action(current.as_ref(), "/synthetic/source", "ext4", "rw"),
        MountAction::Replace
    );
}

#[test]
fn auto_converges_without_commands_and_remounts_using_detected_type() {
    use super::super::{Mounted, apply_mount, find_mount, mount_action};
    for fstype in ["ext4", "xfs"] {
        for options in ["rw,noatime", "ro"] {
            let mounts = format!("/synthetic/source /target {fstype} rw,noatime 0 0\n");
            let old = find_mount(&mounts, "/target");
            let desired = Mounted {
                source: "/synthetic/source".into(),
                fstype: "auto".into(),
                options: options.into(),
            };
            let action = mount_action(old.as_ref(), &desired.source, &desired.fstype, options);
            let mut calls = Vec::new();
            let mut persisted = 0;
            apply_mount(
                "/target",
                old.as_ref(),
                &desired,
                action,
                &mut |program, args| {
                    calls.push((
                        program.to_owned(),
                        args.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    ));
                    Ok(())
                },
                &mut || {
                    persisted += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(persisted, 1);
            let expected = if options == "ro" {
                vec![(
                    "mount".to_owned(),
                    vec![
                        "-t",
                        fstype,
                        "-o",
                        "remount,ro",
                        "--",
                        "/synthetic/source",
                        "/target",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                )]
            } else {
                vec![]
            };
            assert_eq!(calls, expected);
        }
    }
}

#[test]
fn auto_fstab_type_is_persisted_literally() {
    let current = "/synthetic/source /target ext4 rw 0 0\n";
    let (next, changed) =
        super::super::rewrite_fstab(current, "/synthetic/source", "/target", "auto", "rw");
    assert!(changed);
    assert_eq!(next, "/synthetic/source\t/target\tauto\trw\t0\t0\n");
    assert_eq!(
        super::super::rewrite_fstab(&next, "/synthetic/source", "/target", "auto", "rw"),
        (next, false)
    );
}

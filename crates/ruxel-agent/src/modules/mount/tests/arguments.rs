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

#[path = "auto.rs"]
mod auto;

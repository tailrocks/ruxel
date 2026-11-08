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
    let (next, _) = super::rewrite_fstab("", "/dev/a\\b", "/mnt/a\tb", "ext4", "rw");
    assert!(next.contains("/dev/a\\134b\t/mnt/a\\011b"));
    assert_eq!(
        super::rewrite_fstab(&next, "/dev/a\\b", "/mnt/a\tb", "ext4", "rw"),
        (next, false)
    );
}

#[test]
fn fstab_corrects_dump_and_pass_drift_and_converges() {
    for current in [
        "/dev/a /srv xfs rw 1 0\n",
        "/dev/a /srv xfs rw 0 2\n",
        "/dev/a /srv xfs rw 1 2\n",
        "/dev/a /srv xfs rw\n",
        "/dev/a /srv xfs rw 0\n",
    ] {
        let (next, changed) = super::rewrite_fstab(current, "/dev/a", "/srv", "xfs", "rw");
        assert!(changed, "{current:?}");
        assert_eq!(next, "/dev/a\t/srv\txfs\trw\t0\t0\n");
        assert_eq!(
            super::rewrite_fstab(&next, "/dev/a", "/srv", "xfs", "rw"),
            (next, false)
        );
    }
    let current = "/dev/a /srv xfs noatime,rw 0 0 # keep\n";
    assert_eq!(
        super::rewrite_fstab(current, "/dev/a", "/srv", "xfs", "rw,noatime"),
        (current.to_string(), false)
    );
}

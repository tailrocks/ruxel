use super::percentage::{parse, target};

#[test]
fn vg_percentage_repairs_drift_and_converges_at_creation_extent_rounding() {
    for total in [10, 11] {
        let plan = target(50, 8, 4, total).unwrap();
        assert!(plan.changed);
        assert_eq!(plan.extents, 5);
        assert!(!target(50, 20, 4, total).unwrap().changed);
    }
    assert_eq!(target(50, 20, 4, 12).unwrap().extents, 6);
    assert!(target(50, 24, 4, 10).is_err());
}
#[test]
fn percentage_targets_fail_closed_for_invalid_geometry_and_unsupported_bases() {
    for (percent, current, extent, total) in [
        (0, 0, 4, 10),
        (101, 0, 4, 10),
        (50, 1, 4, 10),
        (50, 0, 0, 10),
        (50, 0, 4, 0),
        (1, 0, 4, 1),
        (100, 0, u64::MAX, 2),
    ] {
        assert!(target(percent, current, extent, total).is_err());
    }
    assert_eq!(target(100, 0, 1, u64::MAX).unwrap().extents, u64::MAX);
    for value in [
        "50%FREE",
        "100%LV",
        "100%ORIGIN",
        "50%PVS",
        "50%V",
        "+50%VG",
        "101%VG",
    ] {
        assert!(parse(value).is_err(), "{value}");
    }
    assert_eq!(parse("50%VG").unwrap(), Some(50));
    assert_eq!(parse("50%vg").unwrap(), Some(50));
}
#[cfg(unix)]
#[test]
fn percentage_provider_check_apply_and_rerun_use_verified_extent_target() {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("RUXEL_LVOL_GEOMETRY_CHILD").is_some() {
        let state = std::env::var("RUXEL_LVOL_GEOMETRY_STATE").unwrap();
        let mut ctx = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let params = serde_json::json!({"vg":"synthetic","lv":"data","size":"50%VG"});
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], true);
        assert_eq!(std::fs::read_to_string(&state).unwrap(), "8\n");
        ctx.check_mode = false;
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], true);
        assert_eq!(std::fs::read_to_string(&state).unwrap(), "20\n");
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], false);
        std::fs::write(&state, "24\n").unwrap();
        assert!(
            super::run(&params, &ctx)
                .unwrap_err()
                .contains("refusing to shrink")
        );
        assert_eq!(std::fs::read_to_string(&state).unwrap(), "24\n");
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-lvol-geometry-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for (name, script) in [
        (
            "lvs",
            "#!/bin/sh\ncase \"$*\" in *lv_name*) printf '{\"report\":[{\"lv\":[{\"lv_name\":\"data\"}]}]}' ;; *) /bin/cat \"$RUXEL_LVOL_GEOMETRY_STATE\" ;; esac\n",
        ),
        (
            "vgs",
            "#!/bin/sh\ncase \"$*\" in *vg_extent_size*) printf '4\\n' ;; *vg_extent_count*) printf '11\\n' ;; *) exit 9 ;; esac\n",
        ),
        (
            "lvresize",
            "#!/bin/sh\n[ \"$*\" = '-l 5 /dev/synthetic/data' ] || exit 9\nprintf '20\\n' > \"$RUXEL_LVOL_GEOMETRY_STATE\"\n",
        ),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let state = dir.join("size");
    std::fs::write(&state, "8\n").unwrap();
    let outcome = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("percentage_provider_check_apply_and_rerun_use_verified_extent_target")
        .env("RUXEL_LVOL_GEOMETRY_CHILD", "1")
        .env("RUXEL_LVOL_GEOMETRY_STATE", &state)
        .env("PATH", &dir)
        .status()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(outcome.success());
}

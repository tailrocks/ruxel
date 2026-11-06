#[cfg(unix)]
#[test]
fn discovery_distinguishes_missing_volume_from_failure() {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("RUXEL_LVOL_PROBE_CHILD").is_some() {
        assert!(super::lv_exists("synthetic", "data").unwrap());
        assert!(!super::lv_exists("synthetic", "missing").unwrap());
        assert!(!super::lv_exists("synthetic", "dat").unwrap());
        assert!(super::lv_exists("synthetic", "pool_tdata").unwrap());
        assert!(super::lv_exists("failure", "data").is_err());
        assert!(!super::lv_exists("empty", "data").unwrap());
        for vg in ["invalid", "bad_shape", "bad_row"] {
            assert!(super::lv_exists(vg, "data").is_err());
        }
        let ctx = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let params = serde_json::json!({"vg":"failure", "lv":"data", "size":"100%FREE"});
        assert!(super::run(&params, &ctx).is_err());
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-lvol-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lvs");
    std::fs::write(&path, "#!/bin/sh\n[ \"$1\" = --all ] && [ \"$2\" = --reportformat ] && [ \"$3\" = json ] && [ \"$4\" = --noheadings ] && [ \"$5\" = -o ] && [ \"$6\" = lv_name ] && [ \"$7\" = -- ] || exit 2\n[ \"$8\" != failure ] || exit 5\ncase \"$8\" in\nempty) printf '{\\\"report\\\":[{\\\"lv\\\":[]}]}' ;;\ninvalid) printf 'LVM2_LV_NAME=data' ;;\nbad_shape) printf '{}' ;;\nbad_row) printf '{\\\"report\\\":[{\\\"lv\\\":[{\\\"lv_name\\\":\\\"data\\\"},{}]}]}' ;;\n*) printf '{\\\"report\\\":[{\\\"lv\\\":[{\\\"lv_name\\\":\\\"data\\\"},{\\\"lv_name\\\":\\\"other\\\"},{\\\"lv_name\\\":\\\"[pool_tdata]\\\"}]}]}' ;;\nesac\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("discovery_distinguishes_missing_volume_from_failure")
        .env("RUXEL_LVOL_PROBE_CHILD", "1")
        .env("PATH", &dir)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

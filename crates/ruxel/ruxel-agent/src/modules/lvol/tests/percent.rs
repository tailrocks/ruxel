#[test]
fn percentage_creation_converges_without_reinterpreting_free_space() {
    for check in [false, true] {
        let needs = super::needs_resize(
            "100%FREE",
            || panic!("must not recalculate free-space percentage"),
            || panic!("must not treat percentage as absolute size"),
        )
        .unwrap();
        let changed = super::reconcile_existing(
            false,
            needs,
            check,
            || -> Result<Option<()>, String> { panic!("unexpected filesystem probe") },
            || panic!("unexpected volume resize"),
            |()| -> Result<bool, String> { panic!("unexpected filesystem resize") },
        )
        .unwrap();
        assert!(!changed);
    }
    assert!(super::needs_resize("+100%FREE", || Ok(2), || unreachable!()).unwrap());
    assert!(!super::needs_resize("+100%FREE", || Ok(0), || unreachable!()).unwrap());
}

#[test]
fn percentage_size_validation_rejects_malformed_values() {
    for size in [
        "bogus%",
        "-50%FREE",
        "0%VG",
        "1.2.3%VG",
        "2.5%PVS",
        "4294967295%VG",
        "999999999999999999999%VG",
        "50%NOPE",
        "%FREE",
        "50%FREE%VG",
    ] {
        assert!(
            super::needs_resize(size, || unreachable!(), || unreachable!()).is_err(),
            "{size}"
        );
    }
    for size in [
        "100%FREE",
        "50%VG",
        "2%PVS",
        "100%ORIGIN",
        "100%LV",
        "100%free",
        "50%V",
        "100%FR",
        "10%p",
        "5%or",
    ] {
        assert!(
            !super::needs_resize(size, || unreachable!(), || unreachable!()).unwrap(),
            "{size}"
        );
    }
}

#[cfg(unix)]
#[test]
fn percentage_volume_create_then_reapply() {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("RUXEL_LVOL_TEST_CHILD").is_some() {
        let mut ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        for invalid in ["bogus%", "-50%FREE", "2.5%VG", "0%VG"] {
            ctx.check_mode = true;
            let params = serde_json::json!({"vg": "synthetic", "lv": "data", "size": invalid});
            assert!(super::run(&params, &ctx).is_err(), "{invalid}");
        }
        ctx.check_mode = false;
        let params = serde_json::json!({"vg": "synthetic", "lv": "data", "size": "100%FREE"});
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], true);
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], false);
        ctx.check_mode = true;
        assert_eq!(super::run(&params, &ctx).unwrap()["changed"], false);
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-lvol-percent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, script) in [
        (
            "lvs",
            "#!/bin/sh\n[ -f \"$RUXEL_LVOL_TEST_STATE\" ] || exit 1\nprintf 'data\\n'\n",
        ),
        (
            "lvcreate",
            "#!/bin/sh\n[ \"$1\" = -l ] && [ \"$2\" = 100%FREE ] || exit 1\n: > \"$RUXEL_LVOL_TEST_STATE\"\n",
        ),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("percentage_volume_create_then_reapply")
        .env("RUXEL_LVOL_TEST_CHILD", "1")
        .env("RUXEL_LVOL_TEST_STATE", dir.join("created"))
        .env("PATH", &dir)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

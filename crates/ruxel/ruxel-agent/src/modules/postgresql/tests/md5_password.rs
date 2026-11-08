use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn md5_password_check_and_apply_converge() {
    if std::env::var_os("RUXEL_MD5_TEST_CHILD").is_some() {
        let mut ctx = ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: true,
            environment: vec![],
            become_user: None,
        };
        let good = json!({"name":"fake-role", "password":"fake-password"});
        let drift = json!({"name":"fake-role", "password":"changed-password"});
        assert_eq!(user(&good, &ctx).unwrap()["changed"], false);
        assert_eq!(user(&drift, &ctx).unwrap()["changed"], true);
        ctx.check_mode = false;
        assert_eq!(user(&good, &ctx).unwrap()["changed"], false);
        assert_eq!(user(&drift, &ctx).unwrap()["changed"], true);
        assert_eq!(user(&drift, &ctx).unwrap()["changed"], false);
        ctx.check_mode = true;
        assert_eq!(user(&drift, &ctx).unwrap()["changed"], false);
        shutdown();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-md5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("writes");
    std::fs::write(&script, "#!/bin/sh\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *pg_authid*) if [ -f \"$RUXEL_MD5_TEST_STATE\" ]; then IFS= read -r verifier < \"$RUXEL_MD5_TEST_STATE\"; else verifier=md5f6b97b8f080991d7c3ab49b6083f61e2; fi; printf '%s\\n' \"$verifier\";;\n  *pg_roles*) printf '1\\n';;\n  'ALTER ROLE '*) printf 'alter\\n' >> \"$RUXEL_MD5_TEST_LOG\"; printf 'md51fbfd09b6743e4656596a4f1b40a3ea4\\n' > \"$RUXEL_MD5_TEST_STATE\";;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("md5_password_check_and_apply_converge")
        .env("RUXEL_MD5_TEST_CHILD", "1")
        .env("RUXEL_MD5_TEST_LOG", &log)
        .env("RUXEL_MD5_TEST_STATE", dir.join("verifier"))
        .env("RUXEL_TEST_PSQL_PATH", &script)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "alter\n");
    for password in ["fake-password", "changed-password"] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(password));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(password));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

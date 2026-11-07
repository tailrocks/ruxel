use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn login_identity_is_passed_and_sessions_are_isolated() {
    if std::env::var_os("RUXEL_FAKE_PSQL_CHILD").is_some() {
        exercise_sessions();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-login-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("calls");
    std::fs::write(&script, "#!/bin/sh\nprintf 'PG=%s %s\\n' \"$PGDATABASE\" \"$*\" >> \"$RUXEL_FAKE_PSQL_LOG\"\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *) printf '1\\n';;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("login_identity_is_passed_and_sessions_are_isolated")
        .env("RUXEL_FAKE_PSQL_CHILD", "1")
        .env("RUXEL_FAKE_PSQL_LOG", &log)
        .env("PATH", &dir)
        .status()
        .unwrap();
    assert!(status.success());
    let calls = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("-d dbname='maintenance'"))
            .count(),
        1
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("-d dbname='sample'"))
            .count(),
        1
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("--username --host=synthetic"))
            .count(),
        3
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("--username other"))
            .count(),
        2
    );
    assert!(lines.iter().any(
        |line| line.starts_with("PG=environment_db ") && line.ends_with("--username other -d ")
    ));
    assert!(calls.contains("dbname='dbname=target user=other host=synthetic \\' \\\\'"));
    std::fs::remove_dir_all(dir).unwrap();
}

fn exercise_sessions() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let user = "--host=synthetic";
    db(
        &json!({"name":"sample", "login_db":"maintenance", "login_user":user}),
        &ctx,
    )
    .unwrap();
    super::super::user(
        &json!({"name":"sample", "login_db":"maintenance", "login_user":user}),
        &ctx,
    )
    .unwrap();
    schema(
        &json!({"name":"sample", "login_db":"sample", "login_user":user}),
        &ctx,
    )
    .unwrap();
    privs(&json!({"role":"sample", "login_db":"sample", "type":"database", "objs":"sample", "privs":"CONNECT", "login_user":user}), &ctx).unwrap();
    db(&json!({"name":"sample", "login_user":"other"}), &ctx).unwrap();
    let env_ctx = ExecContext {
        environment: vec![("PGDATABASE".into(), "environment_db".into())],
        check_mode: true,
        diff_mode: false,
        no_log: false,
        become_user: None,
    };
    db(
        &json!({"name":"sample", "login_db":"", "login_user":"other"}),
        &env_ctx,
    )
    .unwrap();
    let database = "dbname=target user=other host=synthetic ' \\";
    db(
        &json!({"name":"sample", "login_db":database, "login_user":user}),
        &ctx,
    )
    .unwrap();
    super::super::user(
        &json!({"name":"sample", "login_db":database, "login_user":user}),
        &ctx,
    )
    .unwrap();
    schema(
        &json!({"name":"sample", "login_db":database, "login_user":user}),
        &ctx,
    )
    .unwrap();
    privs(&json!({"role":"sample", "login_db":database, "type":"database", "objs":"sample", "privs":"CONNECT", "login_user":user}), &ctx).unwrap();
    shutdown();
}

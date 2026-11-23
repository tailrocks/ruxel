use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn task_environment_is_applied_and_isolates_cached_sessions() {
    if std::env::var_os("RUXEL_ENV_TEST_CHILD").is_some() {
        let mut context = ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![("PGHOST".into(), "first".into())],
            become_user: None,
        };
        for (index, credentials) in [
            Some(("first", "synthetic-a")),
            Some(("first", "synthetic-a")),
            Some(("first", "synthetic-a")),
            Some(("first", "synthetic-b")),
            Some(("second", "synthetic-b")),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            context.environment = credentials.map_or_else(Vec::new, |(host, password)| {
                vec![
                    ("PGHOST".into(), host.into()),
                    ("PGPASSWORD".into(), password.into()),
                ]
            });
            if index == 1 {
                context.environment.reverse();
            } else if index == 2 {
                context
                    .environment
                    .insert(0, ("PGHOST".into(), "superseded".into()));
            }
            let pg = PgContext {
                exec: &context,
                login_user: None,
                login_db: None,
            };
            assert_eq!(psql(&pg, "5432", None, "SELECT 1").unwrap(), "1");
        }
        shutdown();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("calls");
    std::fs::write(&script, "#!/bin/sh\ncase \"${PGHOST-unset}:${PGPASSWORD-unset}\" in first:synthetic-a|first:synthetic-b|second:synthetic-b|unset:unset) ;; *) exit 8;; esac\nprintf '%s\\n' \"${PGHOST-unset}\" >> \"$RUXEL_FAKE_PSQL_LOG\"\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *) printf '1\\n';;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("task_environment_is_applied_and_isolates_cached_sessions")
        .env("RUXEL_ENV_TEST_CHILD", "1")
        .env("RUXEL_FAKE_PSQL_LOG", &log)
        .env("RUXEL_TEST_PSQL_PATH", &script)
        .env_remove("PGHOST")
        .env_remove("PGPASSWORD")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "first\nfirst\nsecond\nunset\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

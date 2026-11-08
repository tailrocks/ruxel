use super::*;

#[test]
fn privilege_requests_reject_empty_privileges_and_targets() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    for params in [
        json!({"login_db":"db", "role":"r", "type":"database", "objs":["db"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "objs":["t"], "privs":[]}),
        json!({"login_db":"db", "role":"r", "type":"schema", "objs":[], "privs":["USAGE"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "privs":["SELECT"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "objs":["t"], "privs":["EXECUTE"]}),
        json!({"login_db":"db", "role":"r", "type":"database", "objs":["db"], "privs":["SELECT"]}),
        json!({"login_db":"db", "role":"r", "type":"default_privs", "privs":["SELECT"], "target_roles":[]}),
    ] {
        assert!(
            super::super::privileges::privs(&params, &ctx).is_err(),
            "{params}"
        );
    }
}

#[test]
fn maintain_requires_postgres_17_before_any_acl_query() {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(version) = std::env::var("RUXEL_PG_MAINTAIN_VERSION") {
        let kind = std::env::var("RUXEL_PG_MAINTAIN_KIND").unwrap();
        let check = std::env::var("RUXEL_PG_MAINTAIN_CHECK").unwrap() == "1";
        let ctx = ExecContext {
            check_mode: check,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let params = json!({"login_db":"fixture", "role":"reader", "type":kind, "objs":["t"], "privs":["SELECT", "maintain"]});
        let result = super::super::privileges::privs(&params, &ctx);
        if version == "160000" {
            assert_eq!(
                result.unwrap_err(),
                "postgresql_privs: MAINTAIN requires PostgreSQL 17 or newer"
            );
        } else {
            assert_eq!(result.unwrap()["changed"], true);
        }
        shutdown();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-maintain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("sql");
    std::fs::write(&script, "#!/bin/sh\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *) printf '%s\\n' \"$line\" >> \"$RUXEL_PG_MAINTAIN_LOG\"; case \"$line\" in 'SHOW server_version_num;'*) printf '%s\\n' \"$RUXEL_PG_MAINTAIN_VERSION\";; esac;;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    for version in ["160000", "170000"] {
        for kind in ["table", "default_privs"] {
            for check in ["0", "1"] {
                let _ = std::fs::remove_file(&log);
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("maintain_requires_postgres_17_before_any_acl_query")
                    .env("RUXEL_PG_MAINTAIN_VERSION", version)
                    .env("RUXEL_PG_MAINTAIN_KIND", kind)
                    .env("RUXEL_PG_MAINTAIN_CHECK", check)
                    .env("RUXEL_PG_MAINTAIN_LOG", &log)
                    .env("PATH", &dir)
                    .status()
                    .unwrap();
                assert!(status.success());
                let sql = std::fs::read_to_string(&log).unwrap();
                assert!(sql.contains("SHOW server_version_num;"));
                if version == "160000" {
                    assert_eq!(sql.trim(), "SHOW server_version_num;");
                } else {
                    assert!(sql.contains("SELECT"));
                    assert_eq!(sql.contains("GRANT"), check == "0");
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn list_fields_drive_probes_and_grants() {
    if std::env::var_os("RUXEL_PG_LIST_CHILD").is_some() {
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        for params in [
            json!({"login_db":"fixture","role":"reader","type":"database","objs":["db,one","db2"],"privs":["CONNECT","CREATE"]}),
            json!({"login_db":"fixture","role":"reader","type":"schema","objs":["schema,one"],"privs":["USAGE"]}),
            json!({"login_db":"fixture","role":"reader","type":"table","objs":["table,one"],"privs":["SELECT","UPDATE"]}),
            json!({"login_db":"fixture","role":"reader","type":"default_privs","target_roles":["writer,one","writer2"],"privs":["SELECT"]}),
        ] {
            assert_eq!(privs(&params, &ctx).unwrap()["changed"], true);
        }
        let bad = json!({"login_db":"fixture","role":"reader","type":"table","objs":[3],"privs":["SELECT"]});
        assert!(privs(&bad, &ctx).is_err());
        shutdown();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-lists-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("sql");
    std::fs::write(&script, "#!/bin/sh\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *) printf '%s\\n' \"$line\" >> \"$RUXEL_PG_LIST_LOG\";;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("list_fields_drive_probes_and_grants")
        .env("RUXEL_PG_LIST_CHILD", "1")
        .env("RUXEL_PG_LIST_LOG", &log)
        .env("PATH", &dir)
        .status()
        .unwrap();
    assert!(status.success());
    let sql = std::fs::read_to_string(log).unwrap();
    for expected in [
        "GRANT CONNECT,CREATE ON DATABASE \"db,one\" TO \"reader\"",
        "GRANT CONNECT,CREATE ON DATABASE \"db2\" TO \"reader\"",
        "GRANT USAGE ON SCHEMA \"schema,one\" TO \"reader\"",
        "GRANT SELECT,UPDATE ON TABLE \"public\".\"table,one\" TO \"reader\"",
        "ALTER DEFAULT PRIVILEGES FOR ROLE \"writer,one\"",
        "ALTER DEFAULT PRIVILEGES FOR ROLE \"writer2\"",
    ] {
        assert!(sql.contains(expected), "missing {expected} in {sql}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

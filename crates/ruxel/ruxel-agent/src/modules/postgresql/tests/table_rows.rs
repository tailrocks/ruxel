use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn all_in_schema_handles_pipes_and_converges() {
    if std::env::var_os("RUXEL_ROWS_TEST_CHILD").is_some() {
        let mut ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        };
        let params = json!({"login_db":"synthetic", "role":"reader", "type":"table", "objs":"ALL_IN_SCHEMA", "schema":"schema|part", "privs":"SELECT"});
        assert_eq!(privs(&params, &ctx).unwrap()["changed"], true);
        assert_eq!(privs(&params, &ctx).unwrap()["changed"], false);
        ctx.check_mode = true;
        assert_eq!(privs(&params, &ctx).unwrap()["changed"], false);
        shutdown();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-rows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let state = dir.join("state");
    let bad = dir.join("bad");
    std::fs::write(&script, "#!/bin/sh\nwhile IFS= read -r line; do\n case \"$line\" in\n  '\\echo '*) printf '%s\\n' \"${line#\\\\echo }\";;\n  '\\q') exit 0;;\n  *pg_tables*) printf '%s\\n' '[[\"schema|part\",\"data|part\"]]';;\n  *pg_class*) case \"$line\" in *\"n.nspname='schema|part'\"*\"c.relname='data|part'\"*) if [ -f \"$RUXEL_ROWS_TEST_STATE\" ]; then printf '1\\n'; fi;; *) printf 'bad\\n' > \"$RUXEL_ROWS_TEST_BAD\";; esac;;\n  'GRANT '*) printf 'granted\\n' > \"$RUXEL_ROWS_TEST_STATE\";;\n esac\ndone\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("all_in_schema_handles_pipes_and_converges")
        .env("RUXEL_ROWS_TEST_CHILD", "1")
        .env("RUXEL_ROWS_TEST_STATE", &state)
        .env("RUXEL_ROWS_TEST_BAD", &bad)
        .env("RUXEL_TEST_PSQL_PATH", &script)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(state.exists());
    assert!(!bad.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn structured_rows_preserve_escaped_identifiers_and_reject_invalid_output() {
    let rows = super::super::privileges::parse_table_rows(
        r#"[["schema|part","line\nquote\"|雪"],["s","tab\tname"]]"#,
    )
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("schema|part".into(), "line\nquote\"|雪".into()),
            ("s".into(), "tab\tname".into())
        ]
    );
    assert_eq!(
        super::super::privileges::parse_table_rows("[]").unwrap(),
        vec![]
    );
    assert!(super::super::privileges::parse_table_rows("schema|table").is_err());
    assert!(super::super::privileges::parse_table_rows("[[\"schema\"]]").is_err());
}

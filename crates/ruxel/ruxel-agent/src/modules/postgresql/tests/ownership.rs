use super::super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn database_and_schema_ownership_create_and_repair_converge() {
    if std::env::var_os("RUXEL_PG_OWNER_CHILD").is_some() {
        exercise_ownership();
        return;
    }
    let dir = std::env::temp_dir().join(format!("ruxel-pg-ownership-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("psql");
    let log = dir.join("writes");
    std::fs::write(&script, r#"#!/bin/sh
while IFS= read -r line; do
 case "$line" in
  '\echo '*) printf '%s\n' "${line#\\echo }";;
  '\q') exit 0;;
  'SELECT 1 FROM pg_database '*) [ ! -f "$RUXEL_PG_OWNER_DB" ] || printf '1\n';;
  'SELECT pg_catalog.pg_get_userbyid'*) [ ! -f "$RUXEL_PG_OWNER_DB" ] || { IFS= read -r value < "$RUXEL_PG_OWNER_DB"; printf '%s\n' "$value"; };;
  'SELECT r.rolname FROM pg_namespace '*) [ ! -f "$RUXEL_PG_OWNER_SCHEMA" ] || { IFS= read -r value < "$RUXEL_PG_OWNER_SCHEMA"; printf '%s\n' "$value"; };;
  'CREATE DATABASE '*|'ALTER DATABASE '*) printf '%s\n' "$line" >> "$RUXEL_PG_OWNER_LOG"; case "$line" in *'"owner_a"'*) printf 'owner_a\n' > "$RUXEL_PG_OWNER_DB";; *'"owner_b"'*) printf 'owner_b\n' > "$RUXEL_PG_OWNER_DB";; *) exit 2;; esac;;
  'CREATE SCHEMA '*|'ALTER SCHEMA '*) printf '%s\n' "$line" >> "$RUXEL_PG_OWNER_LOG"; case "$line" in *'"owner_a"'*) printf 'owner_a\n' > "$RUXEL_PG_OWNER_SCHEMA";; *'"owner_b"'*) printf 'owner_b\n' > "$RUXEL_PG_OWNER_SCHEMA";; *) exit 2;; esac;;
  *) exit 2;;
 esac
done
"#).unwrap();
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("database_and_schema_ownership_create_and_repair_converge")
        .env("RUXEL_PG_OWNER_CHILD", "1")
        .env("RUXEL_PG_OWNER_DB", dir.join("db"))
        .env("RUXEL_PG_OWNER_SCHEMA", dir.join("schema"))
        .env("RUXEL_PG_OWNER_LOG", &log)
        .env("PATH", &dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let statements = std::fs::read_to_string(log).unwrap();
    assert_eq!(statements.lines().count(), 4);
    for expected in [
        "CREATE DATABASE \"fixture\" OWNER \"owner_a\"",
        "CREATE SCHEMA \"fixture\" AUTHORIZATION \"owner_a\"",
        "ALTER DATABASE \"fixture\" OWNER TO \"owner_b\"",
        "ALTER SCHEMA \"fixture\" OWNER TO \"owner_b\"",
    ] {
        assert!(statements.contains(expected), "{expected}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn exercise_ownership() {
    let mut ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    for owner in ["owner_a", "owner_b"] {
        let database = json!({"name":"fixture", "owner":owner, "login_db":"maintenance"});
        let namespace = json!({"name":"fixture", "owner":owner, "login_db":"maintenance"});
        assert_eq!(db(&database, &ctx).unwrap()["changed"], true);
        assert_eq!(schema(&namespace, &ctx).unwrap()["changed"], true);
        ctx.check_mode = false;
        assert_eq!(db(&database, &ctx).unwrap()["changed"], true);
        assert_eq!(schema(&namespace, &ctx).unwrap()["changed"], true);
        assert_eq!(db(&database, &ctx).unwrap()["changed"], false);
        assert_eq!(schema(&namespace, &ctx).unwrap()["changed"], false);
        ctx.check_mode = true;
        assert_eq!(db(&database, &ctx).unwrap()["changed"], false);
        assert_eq!(schema(&namespace, &ctx).unwrap()["changed"], false);
    }
    shutdown();
}

use super::*;

#[test]
fn native_operation_kinds_replace_collection_names() -> Result<()> {
    for (kind, legacy, params) in [
        (
            "volume_group",
            "community.general.lvg",
            json!({"vg":"vg", "pvs":"/dev/example"}),
        ),
        (
            "logical_volume",
            "community.general.lvol",
            json!({"vg":"vg", "lv":"lv", "size":"1G"}),
        ),
        (
            "timezone",
            "community.general.timezone",
            json!({"name":"UTC"}),
        ),
        (
            "mount",
            "ansible.posix.mount",
            json!({"path":"/tmp/mount", "src":"/dev/example", "fstype":"ext4"}),
        ),
        (
            "postgres_database",
            "community.postgresql.postgresql_db",
            json!({"name":"app"}),
        ),
        (
            "postgres_user",
            "community.postgresql.postgresql_user",
            json!({"name":"app"}),
        ),
        (
            "postgres_schema",
            "community.postgresql.postgresql_schema",
            json!({"name":"app", "login_db":"db"}),
        ),
        (
            "postgres_privileges",
            "community.postgresql.postgresql_privs",
            json!({"login_db":"db", "role":"app", "type":"default_privs"}),
        ),
    ] {
        let params = params.as_object().context("operation params")?;
        validate_operation(kind, params)?;
        assert!(validate_operation(legacy, params).is_err(), "{legacy}");
    }
    let invalid = json!({"login_db":"db", "role":"app", "type":"table", "target_roles":"creator"});
    assert!(
        validate_operation(
            "postgres_privileges",
            invalid.as_object().context("params")?
        )
        .is_err()
    );
    Ok(())
}

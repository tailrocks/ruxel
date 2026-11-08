use super::*;

#[test]
fn groups_and_pvs_require_lists_before_dispatch() -> Result<()> {
    for (kind, field, base) in [
        ("user", "groups", json!({"name":"fixture"})),
        ("volume_group", "pvs", json!({"vg":"fixture"})),
    ] {
        for value in [json!(["one", "two"])] {
            let mut params = base.clone();
            params[field] = value;
            validate_operation(kind, params.as_object().context("params")?)?;
        }
        for value in [
            json!("one,two"),
            json!(["one", 2]),
            json!({"kind":"ref", "path":["vars", "values"]}),
        ] {
            let mut params = base.clone();
            params[field] = value;
            let params = params.as_object().context("params")?;
            assert!(validate_operation(kind, params).is_err());
            if params[field].is_object() {
                operation(kind, params, true)?;
            } else {
                assert!(operation(kind, params, true).is_err());
            }
        }
    }
    let groups = json!({"name":"fixture", "groups":[]});
    validate_operation("user", groups.as_object().context("groups")?)?;
    let pvs = json!({"vg":"fixture", "pvs":[]});
    assert!(validate_operation("volume_group", pvs.as_object().context("pvs")?).is_err());
    assert!(operation("volume_group", pvs.as_object().context("pvs")?, true).is_err());
    Ok(())
}

#[test]
fn native_operation_kinds_replace_collection_names() -> Result<()> {
    for (kind, legacy, params) in [
        (
            "volume_group",
            "community.general.lvg",
            json!({"vg":"vg", "pvs":["/dev/example"]}),
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

use super::{Result, evaluate};

#[test]
fn user_groups_and_volume_pvs_are_lists_or_expressions() -> Result {
    for (class, base, field) in [
        ("User", "name = \"fixture\"", "groups"),
        ("VolumeGroup", "vg = \"fixture\"", "pvs"),
    ] {
        for value in [r#""one,two""#, "42", "Listing(\"one\", 2)"] {
            assert!(
                evaluate(&format!("new R.{class} {{ {base}; {field} = {value} }}")).is_err(),
                "{class} {value}"
            );
        }
        for value in [
            r#"Listing("one", "two")"#,
            r#"new R.Ref { path { "vars"; "values" } }"#,
        ] {
            evaluate(&format!("new R.{class} {{ {base}; {field} = {value} }}"))?;
        }
    }
    assert!(evaluate(r#"new R.VolumeGroup { vg = "fixture"; pvs = Listing() }"#).is_err());
    evaluate(r#"new R.User { name = "fixture"; groups = Listing() }"#)?;
    Ok(())
}

/// `groups { ... }` and `pvs { ... }` amend each union's `*Listing` default,
/// as stock `pkl eval` requires: a union without a `*` default, or an explicit
/// `= null` default, cannot be amended there.
#[test]
fn user_groups_and_volume_pvs_accept_listing_bodies() -> Result {
    let user = evaluate(r#"new R.User { name = "fixture"; groups { "adm"; "sudo" } }"#)?;
    assert_eq!(user["groups"], serde_json::json!(["adm", "sudo"]));
    let unset = evaluate(r#"new R.User { name = "fixture" }"#)?;
    assert!(
        unset.get("groups").is_none_or(serde_json::Value::is_null),
        "{unset}"
    );
    let volume = evaluate(r#"new R.VolumeGroup { vg = "fixture"; pvs { "/dev/sdb" } }"#)?;
    assert_eq!(volume["pvs"], serde_json::json!(["/dev/sdb"]));
    assert!(evaluate(r#"new R.VolumeGroup { vg = "fixture" }"#).is_err());
    assert!(evaluate(r#"new R.VolumeGroup { vg = "fixture"; pvs {} }"#).is_err());
    Ok(())
}

/// `TextList` and `PrivilegeList` amend their `*Listing` default too. An unset
/// optional `TextList` stays null, and `privs` stays required and non-empty.
#[test]
fn text_and_privilege_lists_accept_listing_bodies() -> Result {
    let apt = evaluate(r#"new R.Apt { name { "curl"; "git" } }"#)?;
    assert_eq!(apt["name"], serde_json::json!(["curl", "git"]));
    let unset = evaluate("new R.Apt {}")?;
    assert!(
        unset.get("name").is_none_or(serde_json::Value::is_null),
        "{unset}"
    );
    let base = r#"login_db = "db"; role = "a""#;
    let privileges = evaluate(&format!(
        r#"new R.PostgresPrivileges {{ {base}; type = "default_privs"; privs {{ "SELECT" }}; objs {{ "tables" }}; target_roles {{ "owner" }} }}"#
    ))?;
    assert_eq!(privileges["privs"], serde_json::json!(["SELECT"]));
    assert_eq!(privileges["objs"], serde_json::json!(["tables"]));
    assert_eq!(privileges["target_roles"], serde_json::json!(["owner"]));
    for privs in ["", "privs {}", r#"privs { " " }"#] {
        assert!(
            evaluate(&format!(
                r#"new R.PostgresPrivileges {{ {base}; type = "table"; {privs} }}"#
            ))
            .is_err(),
            "{privs}"
        );
    }
    assert!(
        evaluate(&format!(
            r#"new R.PostgresPrivileges {{ {base}; type = "database"; privs = "ALL" }}"#
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn become_user_requires_nonempty_valid_usernames() -> Result {
    for class in ["Play", "Task"] {
        let operation = if class == "Task" {
            "operation = new R.Debug {};"
        } else {
            ""
        };
        for user in ["", " ", "bad user", "-root", "root\n", "a/b", "é"] {
            let user = serde_json::to_string(user)?;
            assert!(
                evaluate(&format!(
                    r#"new R.{class} {{ name = "fixture"; {operation} becomeUser = {user} }}"#
                ))
                .is_err(),
                "{class}: {user}"
            );
        }
        for user in ["null", r#""root""#, r#""_service""#, r#""svc-user.1""#] {
            evaluate(&format!(
                r#"new R.{class} {{ name = "fixture"; {operation} becomeUser = {user} }}"#
            ))?;
        }
    }
    Ok(())
}

#[test]
fn file_recurse_requires_directory_or_deferred_state() -> Result {
    for state in [
        "",
        r#"state = "file";"#,
        r#"state = "absent";"#,
        r#"state = "link"; src = "/tmp/source";"#,
    ] {
        assert!(
            evaluate(&format!(
                r#"new R.File {{ path = "/tmp/fixture"; {state} recurse = true }}"#
            ))
            .is_err(),
            "{state}"
        );
        for recurse in [
            "false",
            "null",
            r#"new R.Ref { path { "vars"; "recursive" } }"#,
        ] {
            evaluate(&format!(
                r#"new R.File {{ path = "/tmp/fixture"; {state} recurse = {recurse} }}"#
            ))?;
        }
    }
    for fields in [
        r#"state = "directory"; recurse = true"#,
        r#"state = "directory"; recurse = false"#,
        r#"state = "directory"; recurse = new R.Ref { path { "vars"; "recursive" } }"#,
        r#"state = new R.Ref { path { "vars"; "state" } }; recurse = true"#,
    ] {
        evaluate(&format!(
            r#"new R.File {{ path = "/tmp/fixture"; {fields} }}"#
        ))?;
    }
    Ok(())
}

#[test]
fn file_requires_canonical_path() -> Result {
    for expression in [
        "new R.File {}",
        r#"new R.File { dest = "/tmp/link" }"#,
        r#"new R.File { path = "/tmp/link"; dest = "/tmp/other" }"#,
        r#"new R.File { path = "/tmp/link"; state = "link" }"#,
    ] {
        assert!(evaluate(expression).is_err(), "{expression}");
    }
    for path in [
        r#""/tmp/link""#,
        r#"new R.Ref { path { "vars"; "target" } }"#,
    ] {
        let value = evaluate(&format!(
            r#"new R.File {{ path = {path}; state = "link"; src = "/tmp/source" }}"#
        ))?;
        assert!(!value.as_object().ok_or("file object")?.contains_key("dest"));
    }
    Ok(())
}

#[test]
fn blocks_reject_result_controls() -> Result {
    let child = r#"new R.Task { name = "child"; operation = new R.Debug {} }"#;
    for control in [r#"register = "result""#, "ignoreErrors = true"] {
        assert!(
            evaluate(&format!(
                r#"new R.Task {{ name = "block"; {control}; block {{ {child} }} }}"#
            ))
            .is_err()
        );
        evaluate(&format!(
            r#"new R.Task {{ name = "leaf"; {control}; operation = new R.Debug {{}} }}"#
        ))?;
    }
    evaluate(&format!(
        r#"new R.Task {{ name = "block"; register = null; ignoreErrors = false; block {{ {child} }} }}"#
    ))?;
    Ok(())
}

/// `that { ... }` amends Assert's `*Listing` default; `that` stays required.
#[test]
fn assert_that_accepts_a_listing_body() -> Result {
    let assert = evaluate("new R.Assert { that { true; false } }")?;
    assert_eq!(assert["that"], serde_json::json!([true, false]));
    assert!(evaluate("new R.Assert {}").is_err());
    assert!(evaluate("new R.Assert { that {} }").is_err());
    Ok(())
}

/// `delegateAllow { ... }` amends a `Listing`: the property has no explicit
/// `= null`, which pkl would amend as a `Dynamic`, yet it stays null when unset.
#[test]
fn delegate_allow_accepts_a_listing_body() -> Result {
    let task = evaluate(
        r#"new R.Task { name = "t"; operation = new R.Debug {}; delegateTo = "h"; delegateAllow { "a" } }"#,
    )?;
    assert_eq!(task["delegateAllow"], serde_json::json!(["a"]));
    let unset = evaluate(r#"new R.Task { name = "t"; operation = new R.Debug {} }"#)?;
    assert!(
        unset
            .get("delegateAllow")
            .is_none_or(serde_json::Value::is_null),
        "{unset}"
    );
    assert!(
        evaluate(r#"new R.Task { name = "t"; operation = new R.Debug {}; delegateAllow { "a" } }"#)
            .is_err()
    );
    Ok(())
}

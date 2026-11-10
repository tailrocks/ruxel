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

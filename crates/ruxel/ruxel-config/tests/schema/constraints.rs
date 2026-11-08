use super::{Result, evaluate};
use serde_json::json;

#[test]
fn delegation_authorization_requires_a_target_on_the_same_task() -> Result {
    for property in [
        "delegateAllow = new Listing { \"a\" }",
        "delegateLocalhost = false",
    ] {
        assert!(
            evaluate(&format!(
                r#"new R.Task {{ name = "child"; operation = new R.Debug {{}}; {property} }}"#
            ))
            .is_err()
        );
    }
    evaluate(
        r#"new R.Task { name = "delegate"; operation = new R.Debug {}; delegateTo = "localhost"; delegateAllow = new Listing { "localhost" }; delegateLocalhost = true }"#,
    )?;
    Ok(())
}

#[test]
fn onepassword_lookup_rejects_conflicting_selector_forms() -> Result {
    for field in ["vault", "item", "section", "field"] {
        assert!(evaluate(&format!(r#"new R.Lookup {{ provider = "op"; reference = "op://vault/item/field"; {field} = "other" }}"#)).is_err(), "{field}");
    }
    evaluate(r#"new R.Lookup { provider = "op"; reference = "op://vault/item/field" }"#)?;
    evaluate(r#"new R.Lookup { provider = "op"; item = "fixture"; vault = "fixture" }"#)?;
    Ok(())
}

#[test]
fn rescue_and_always_require_block_tasks() -> Result {
    let child = r#"new R.Task { name = "cleanup"; operation = new R.Debug {} }"#;
    for section in ["rescue", "always"] {
        assert!(evaluate(&format!(r#"new R.Task {{ name = "run"; operation = new R.Debug {{}}; {section} {{ {child} }} }}"#)).is_err(), "{section}");
        evaluate(&format!(
            r#"new R.Task {{ name = "block"; block {{ {child} }}; {section} {{ {child} }} }}"#
        ))?;
    }
    Ok(())
}

#[test]
fn loop_variable_cannot_shadow_scope_roots() -> Result {
    for name in [
        "vars",
        "variables",
        "facts",
        "results",
        "inventoryHost",
        "setFacts",
    ] {
        assert!(evaluate(&format!(r#"new R.Task {{ name = "loop"; loopVar = "{name}"; operation = new R.Debug {{}} }}"#)).is_err(), "{name}");
    }
    for name in ["item", "entry"] {
        evaluate(&format!(
            r#"new R.Task {{ name = "loop"; loopVar = "{name}"; operation = new R.Debug {{}} }}"#
        ))?;
    }
    Ok(())
}

#[test]
fn apt_repository_requires_filename() -> Result {
    assert!(evaluate(r#"new R.AptRepository { repo = "deb fixture" }"#).is_err());
    evaluate(r#"new R.AptRepository { repo = "deb fixture"; filename = "fixture" }"#)?;
    evaluate(
        r#"new R.AptRepository { repo = "deb fixture"; filename = new R.Ref { path { "vars"; "filename" } } }"#,
    )?;
    Ok(())
}

#[test]
fn set_fact_always_emits_values_mapping() -> Result {
    assert_eq!(evaluate("new R.SetFact {}")?["values"], json!({}));
    assert_eq!(
        evaluate(r#"new R.SetFact { values { ["answer"] = new R.Literal { value = 42 } } }"#)?["values"]
            ["answer"]["value"],
        42
    );
    Ok(())
}

#[test]
fn postgres_maintenance_database_selector() -> Result {
    for kind in ["PostgresDatabase", "PostgresRole"] {
        assert_eq!(
            evaluate(&format!(
                r#"new R.{kind} {{ name = "fixture"; login_db = "maintenance" }}"#
            ))?["login_db"],
            "maintenance"
        );
    }
    Ok(())
}

#[test]
fn blocks_reject_loop_controls() -> Result {
    let child = r#"new R.Task { name = "child"; operation = new R.Debug {} }"#;
    for control in [
        r"`loop` = new R.ListExpr {}",
        r#"loopLabel = new R.Literal { value = "label" }"#,
        r#"loopVar = "entry""#,
    ] {
        assert!(
            evaluate(&format!(
                r#"new R.Task {{ name = "block"; {control}; block {{ {child} }} }}"#
            ))
            .is_err(),
            "{control}"
        );
    }
    evaluate(
        r#"new R.Task { name = "operation"; `loop` = new R.ListExpr {}; loopVar = "entry"; loopLabel = new R.Literal { value = "label" }; operation = new R.Debug {} }"#,
    )?;
    Ok(())
}

#[test]
fn retries_distinguish_omitted_zero_and_invalid_negative() -> Result {
    assert!(evaluate(r#"new R.Task { name = "bad"; retries = 0 }"#).is_err());
    for retries in ["", "retries = 0;", "retries = 1;"] {
        let value = evaluate(&format!(
            r#"new R.Task {{ name = "retry"; {retries} operation = new R.Debug {{}} }}"#
        ))?;
        assert_eq!(
            value["retries"],
            match retries {
                "" => json!(null),
                "retries = 0;" => json!(0),
                _ => json!(1),
            }
        );
    }
    assert!(
        evaluate(r#"new R.Task { name = "bad"; retries = -1; operation = new R.Debug {} }"#)
            .is_err()
    );
    Ok(())
}

mod controls;

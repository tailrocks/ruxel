use super::{Result, evaluate};

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

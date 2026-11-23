use super::*;

mod dynamic;

#[test]
fn file_recurse_requires_directory_before_and_after_expression_resolution() -> Result<()> {
    for state in [Value::Null, json!("file"), json!("absent"), json!("link")] {
        let fields =
            json!({"path":"/tmp/fixture","src":"/tmp/source","state":state,"recurse":true});
        let mut operation = fields.clone();
        operation["kind"] = json!("file");
        let error = validate_config(&config(operation)).expect_err("invalid recurse state");
        assert!(
            error
                .to_string()
                .contains("recurse requires state=directory"),
            "{error:#}"
        );
        let error = validate_operation("file", fields.as_object().context("params")?)
            .expect_err("invalid resolved recurse state");
        assert!(
            error
                .to_string()
                .contains("recurse requires state=directory"),
            "{error:#}"
        );
        for recurse in [Value::Null, json!(false)] {
            let mut fields = fields.clone();
            fields["recurse"] = recurse;
            validate_operation("file", fields.as_object().context("params")?)?;
            fields["kind"] = json!("file");
            validate_config(&config(fields))?;
        }
    }
    for recurse in [json!(true), json!(false)] {
        let fields = json!({"path":"/tmp/fixture","state":"directory","recurse":recurse});
        validate_operation("file", fields.as_object().context("params")?)?;
        let mut operation = fields;
        operation["kind"] = json!("file");
        validate_config(&config(operation))?;
    }
    for (state, recurse) in [
        (json!({"kind":"ref","path":["vars","state"]}), json!(true)),
        (
            json!("file"),
            json!({"kind":"ref","path":["vars","recursive"]}),
        ),
    ] {
        validate_config(&config(
            json!({"kind":"file","path":"/tmp/fixture","state":state,"recurse":recurse}),
        ))?;
    }
    Ok(())
}

#[test]
fn file_requires_path_and_rejects_destination_alias() -> Result<()> {
    for fields in [
        json!({}),
        json!({"dest":"/tmp/file"}),
        json!({"path":"/tmp/file", "dest":"/tmp/other"}),
        json!({"path":null}),
        json!({"path":"/tmp/link", "state":"link"}),
    ] {
        let mut operation = fields.clone();
        operation["kind"] = json!("file");
        assert!(validate_config(&config(operation)).is_err(), "{fields}");
        assert!(validate_operation("file", fields.as_object().context("params")?).is_err());
    }
    for path in [
        json!("/tmp/file"),
        json!({"kind":"ref", "path":["vars","target"]}),
    ] {
        validate_config(&config(json!({"kind":"file", "path":path})))?;
    }
    validate_operation(
        "file",
        json!({"path":"/tmp/link", "state":"link", "src":"/tmp/file"})
            .as_object()
            .context("params")?,
    )?;
    Ok(())
}

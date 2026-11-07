use super::*;

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

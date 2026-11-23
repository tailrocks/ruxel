use super::{Format, RenderError, render};
use serde_json::json;

#[test]
fn json_object_is_pretty_and_terminated() -> Result<(), RenderError> {
    let rendered = render(Format::Json, &json!({"name": "ruxel", "enabled": true}));
    assert_eq!(
        rendered?,
        "{\n  \"name\": \"ruxel\",\n  \"enabled\": true\n}\n"
    );
    Ok(())
}

#[test]
fn yaml_single_document_is_pretty_json() -> Result<(), RenderError> {
    assert_eq!(
        render(Format::Yaml, &json!({"name": "ruxel"}))?,
        "{\n  \"name\": \"ruxel\"\n}\n"
    );
    Ok(())
}

#[test]
fn yaml_array_is_a_sequence_of_documents() -> Result<(), RenderError> {
    assert_eq!(
        render(Format::Yaml, &json!([{"name": "one"}, {"name": "two"}]))?,
        "{\n  \"name\": \"one\"\n}\n---\n{\n  \"name\": \"two\"\n}\n"
    );
    Ok(())
}

#[test]
fn toml_renders_nested_table_and_drops_nulls() -> Result<(), RenderError> {
    assert_eq!(
        render(
            Format::Toml,
            &json!({"name": "ruxel", "optional": null, "nested": {"enabled": true, "ignored": null}}),
        )?,
        "name = \"ruxel\"\n\n[nested]\nenabled = true\n"
    );
    Ok(())
}

#[test]
fn toml_rejects_non_object_root() {
    assert!(matches!(
        render(Format::Toml, &json!([1, 2])),
        Err(RenderError::TomlRoot)
    ));
}

#[test]
fn key_order_is_preserved_in_every_format() -> Result<(), RenderError> {
    let value = json!({"b": 1, "a": 2});
    for format in [Format::Json, Format::Yaml, Format::Toml] {
        let rendered = render(format, &value)?;
        let expected_prefix = match format {
            Format::Json | Format::Yaml => "{\n  \"b\": 1,\n  \"a\": 2",
            Format::Toml => "b = 1\na = 2",
        };
        assert!(rendered.starts_with(expected_prefix), "{rendered}");
    }
    Ok(())
}

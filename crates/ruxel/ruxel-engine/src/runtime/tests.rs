use super::*;

#[test]
fn redaction_preserves_json_structure_for_short_and_escaped_secrets() -> Result<()> {
    let lookups = Lookups {
        resolver: Resolver::synthetic(),
        values: Mutex::new(vec!["true".into(), "quoted\"secret".into()]),
    };
    let result =
        lookups.redact(&json!({"value":true,"nested":["true","quoted\"secret"],"true-key":1}))?;
    assert_eq!(result["value"], true);
    assert_eq!(result["nested"], json!(["[REDACTED]", "[REDACTED]"]));
    assert_eq!(result["[REDACTED]-key"], 1);
    Ok(())
}

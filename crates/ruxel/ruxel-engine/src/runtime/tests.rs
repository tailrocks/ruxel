use super::*;

mod prefetch;

#[test]
fn sensitive_event_exposes_only_safe_metadata() -> Result<()> {
    let projected = output::sensitive_event(
        &json!({"host":"synthetic-marker", "task":"synthetic-marker", "item":"synthetic-marker", "pause":"synthetic-marker", "result":{"msg":"synthetic-marker", "changed":true, "failed":false}}),
    );
    assert!(!serde_json::to_string(&projected)?.contains("synthetic-marker"));
    assert_eq!(projected["host"], "[censored]");
    assert_eq!(projected["task"], "[censored]");
    assert_eq!(projected["result"]["changed"], true);
    assert_eq!(projected["result"]["failed"], false);
    Ok(())
}

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

mod template_source;

#[test]
fn private_event_log_records_redacted_structure_and_refuses_overwrite() -> Result<()> {
    let path = std::env::temp_dir().join(format!("ruxel-events-{}.jsonl", std::process::id()));
    let lookups = Lookups {
        resolver: Resolver::synthetic(),
        values: Mutex::new(vec!["synthetic-private-value".into()]),
    };
    let output = Output::new(OutputFormat::Human, Some(&path))?;
    output.event(&lookups.redact(
        &json!({"host":"fixture","result":{"value":"synthetic-private-value","failed":false}}),
    )?)?;
    let text = std::fs::read_to_string(&path)?;
    let value: Value = serde_json::from_str(&text)?;
    assert_eq!(value["result"]["value"], "[REDACTED]");
    assert_eq!(value["result"]["failed"], false);
    assert!(Output::new(OutputFormat::Json, Some(&path)).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    drop(output);
    std::fs::remove_file(path)?;
    Ok(())
}

use super::*;
use crate::expression::percent_encode;
use base64::{Engine as _, engine::general_purpose::STANDARD};

mod fetch_secret;
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
fn redaction_preserves_json_structure_for_short_and_escaped_secrets() {
    let lookups = Lookups::with_values(
        Resolver::synthetic().expect("synthetic resolver"),
        vec!["true".into(), "quoted\"secret".into()],
    );
    let result =
        lookups.redact(&json!({"value":true,"nested":["true","quoted\"secret"],"true-key":1}));
    assert_eq!(result["value"], true);
    assert_eq!(result["nested"], json!(["[REDACTED]", "[REDACTED]"]));
    assert_eq!(result["[REDACTED]-key"], 1);
}

mod template_source;

#[test]
fn private_event_log_records_redacted_structure_and_refuses_overwrite() -> Result<()> {
    let path = std::env::temp_dir().join(format!("ruxel-events-{}.jsonl", std::process::id()));
    let lookups = Lookups::with_values(
        Resolver::synthetic().expect("synthetic resolver"),
        vec!["synthetic-private-value".into()],
    );
    let output = Output::new(OutputFormat::Human, Some(&path))?;
    output.event(&lookups.redact(
        &json!({"host":"fixture","result":{"value":"synthetic-private-value","failed":false}}),
    ))?;
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

/// The captured output of a run carries no secret value in any form: the
/// exact value, its interpolation into text, its join with another secret,
/// and its standard and URL-safe base64, its lower- and uppercase hex and
/// its percent-encoded (`urlencode`) forms are all redacted from every
/// event written to stdout and the event log. The test greps the captured
/// output for the plaintext and fails on any hit.
#[test]
fn captured_output_carries_no_plaintext_in_any_derived_form() -> Result<()> {
    // The space and `+` are percent-encoded by the `urlencode` transform, so
    // the encoded form is a distinct string the redactor must also match.
    const SECRET: &str = "gravy training+7f3a";
    const OTHER: &str = "kubeadm-pair-9c21";
    let lookups = Lookups::with_values(Resolver::synthetic().expect("synthetic resolver"), vec![]);
    lookups.record(SECRET);
    lookups.record(OTHER);
    let secret_b64 = STANDARD.encode(SECRET);
    let secret_hex: String = SECRET
        .bytes()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("hex digit"))
        .collect();
    let secret_hex_upper = secret_hex.to_uppercase();
    let secret_percent = percent_encode(SECRET);
    let events = vec![
        json!({"host":"fixture","task":"Write the env file","result":{
            "changed": true,
            "interpolated": format!("PASSWORD={SECRET}\n"),
            "joined": format!("{SECRET}.{OTHER}"),
            "encoded": secret_b64,
            "hex": secret_hex.clone(),
            "hex_upper": secret_hex_upper.clone(),
            "urlsafe": format!("token {}", secret_b64.replace('+', "-").replace('/', "_")),
            "percent": format!("query {}", secret_percent.clone()),
        }}),
        json!({"host":"fixture","error":format!("lookup failed near {SECRET}")}),
    ];
    let path =
        std::env::temp_dir().join(format!("ruxel-no-plaintext-{}.jsonl", std::process::id()));
    drop(std::fs::remove_file(&path));
    let output = Output::new(OutputFormat::Json, Some(&path))?;
    let mut stdout = Vec::new();
    for event in &events {
        let redacted = lookups.redact(event);
        // stdout capture: the JSON line the controller would print.
        stdout.extend_from_slice(format!("{redacted}\n").as_bytes());
        output.event(&redacted)?;
    }
    drop(output);
    let captured = String::from_utf8(stdout)? + &std::fs::read_to_string(&path)?;
    drop(std::fs::remove_file(&path));
    for plaintext in [
        SECRET,
        OTHER,
        format!("{SECRET}.{OTHER}").as_str(),
        format!("PASSWORD={SECRET}").as_str(),
        secret_b64.as_str(),
        secret_b64.trim_end_matches('='),
        secret_b64.replace('+', "-").replace('/', "_").as_str(),
        secret_hex.as_str(),
        secret_hex_upper.as_str(),
        secret_percent.as_str(),
    ] {
        assert!(
            !captured.contains(plaintext),
            "captured output carries plaintext: {plaintext}"
        );
    }
    // The event log carries the structure and the safe fields, so the grep
    // ran over real captured output, not an empty file.
    assert!(captured.contains("[REDACTED]"));
    assert!(captured.contains("changed"));
    Ok(())
}

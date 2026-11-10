use super::{
    FINGERPRINT_PROPERTY, PLACEHOLDER, SERVED_FINGERPRINT, SOURCE, admit, exported_source,
    fingerprint, source,
};
use serde_json::json;

#[test]
fn served_and_exported_schemas_replace_the_single_placeholder() {
    assert_eq!(SOURCE, include_str!("../../../../../schema/Ruxel.pkl"));
    assert_eq!(SOURCE.matches(PLACEHOLDER).count(), 1);
    assert!(fingerprint().starts_with("blake3:"));
    let served = SERVED_FINGERPRINT.as_str();
    assert!(served.starts_with(&format!("{}@", fingerprint())));
    assert!(source().contains(&format!(
        "fixed {FINGERPRINT_PROPERTY}: String = \"{served}\""
    )));
    assert!(exported_source().contains(&format!(
        "fixed {FINGERPRINT_PROPERTY}: String = \"{}\"",
        fingerprint()
    )));
    for text in [source(), exported_source()] {
        assert_eq!(text.matches(PLACEHOLDER).count(), 0);
    }
}

#[test]
fn canonical_schema_parses_without_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    let tokens = pklr::lexer::lex(source())?;
    pklr::parser::parse_named(&tokens, source(), "Ruxel.pkl")?;
    Ok(())
}

#[test]
fn admission_strips_the_served_fingerprint_at_the_root_or_under_config() {
    let served = SERVED_FINGERPRINT.as_str();
    for (mut value, expected) in [
        (
            json!({"plays": [], FINGERPRINT_PROPERTY: served}),
            json!({"plays": []}),
        ),
        (
            json!({"config": {"plays": [], FINGERPRINT_PROPERTY: served}}),
            json!({"config": {"plays": []}}),
        ),
    ] {
        assert!(admit(&mut value).is_ok());
        assert_eq!(value, expected);
    }
}

#[test]
fn admission_refuses_copies_and_names_the_fix() {
    let other_hash = format!("blake3:{}", "a".repeat(64));
    let other_nonce = format!("{}@{}", fingerprint(), "0".repeat(32));
    for (found, shown) in [
        (None, "no Ruxel schema fingerprint".to_owned()),
        (Some(json!("unserved")), "unserved".to_owned()),
        (Some(json!(other_hash.clone())), other_hash.clone()),
        // An unmodified copy, or one edited without changing the string.
        (
            Some(json!(fingerprint())),
            "a copy of this ruxel's schema".to_owned(),
        ),
        (
            Some(json!(other_nonce)),
            "a copy of this ruxel's schema".to_owned(),
        ),
        // Configuration-chosen text is never echoed.
        (
            Some(json!("secret-value")),
            "an unrecognized fingerprint".to_owned(),
        ),
        (Some(json!(42)), "an unrecognized fingerprint".to_owned()),
    ] {
        let mut value = json!({"config": {"plays": []}});
        if let Some(found) = found {
            value["config"][FINGERPRINT_PROPERTY] = found;
        }
        let Err(error) = admit(&mut value) else {
            panic!("{shown}: admitted");
        };
        let error = error.to_string();
        assert!(error.contains(&shown), "{error}");
        assert!(!error.contains("secret-value"), "{error}");
        assert!(error.contains(fingerprint()), "{error}");
        assert!(error.contains("modulepath:/ruxel/Ruxel.pkl"), "{error}");
        assert!(error.contains("ruxel schema export"), "{error}");
    }
}

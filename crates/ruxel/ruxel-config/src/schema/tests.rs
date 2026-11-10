use super::{FINGERPRINT_PROPERTY, PLACEHOLDER, SOURCE, admit, fingerprint, source};
use serde_json::json;

#[test]
fn served_schema_replaces_the_single_placeholder_with_its_fingerprint() {
    assert_eq!(SOURCE, include_str!("../../../../../schema/Ruxel.pkl"));
    assert_eq!(SOURCE.matches(PLACEHOLDER).count(), 1);
    assert!(fingerprint().starts_with("blake3:"));
    assert_eq!(source().matches(PLACEHOLDER).count(), 0);
    assert_eq!(
        source().matches(&format!("\"{}\"", fingerprint())).count(),
        1
    );
    assert!(source().contains(&format!(
        "fixed {FINGERPRINT_PROPERTY}: String = \"{}\"",
        fingerprint()
    )));
}

#[test]
fn canonical_schema_parses_without_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    let tokens = pklr::lexer::lex(source())?;
    pklr::parser::parse_named(&tokens, source(), "Ruxel.pkl")?;
    Ok(())
}

#[test]
fn admission_strips_a_matching_fingerprint_at_the_root_or_under_config() {
    for (mut value, expected) in [
        (
            json!({"plays": [], FINGERPRINT_PROPERTY: fingerprint()}),
            json!({"plays": []}),
        ),
        (
            json!({"config": {"plays": [], FINGERPRINT_PROPERTY: fingerprint()}}),
            json!({"config": {"plays": []}}),
        ),
    ] {
        assert!(admit(&mut value).is_ok());
        assert_eq!(value, expected);
    }
}

#[test]
fn admission_names_both_schemas_and_the_fix() {
    for (mut value, found) in [
        (
            json!({"config": {"plays": []}}),
            "no Ruxel schema fingerprint",
        ),
        (
            json!({"config": {"plays": [], FINGERPRINT_PROPERTY: "unserved"}}),
            "unserved",
        ),
        (
            json!({"plays": [], FINGERPRINT_PROPERTY: "blake3:00"}),
            "blake3:00",
        ),
    ] {
        let Err(error) = admit(&mut value) else {
            panic!("{found}: admitted");
        };
        let error = error.to_string();
        assert!(error.contains(found), "{error}");
        assert!(error.contains(fingerprint()), "{error}");
        assert!(error.contains("modulepath:/ruxel/Ruxel.pkl"), "{error}");
        assert!(error.contains("ruxel schema export"), "{error}");
    }
}

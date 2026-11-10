use super::super::{
    ENGINE, FINGERPRINT_PROPERTY, FINGERPRINTED, LANGUAGE_FINGERPRINT_PROPERTY, PLACEHOLDER,
    SERVED_FINGERPRINT, TREE, admit, exported_modules, exported_source, fingerprint, modules,
    source,
};
use serde_json::json;

#[test]
fn served_and_exported_engine_replace_the_single_placeholder() {
    let committed = TREE
        .iter()
        .find(|(path, _)| *path == ENGINE)
        .map_or("", |(_, text)| *text);
    assert_eq!(committed.matches(PLACEHOLDER).count(), 1);
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
    for (path, committed) in TREE {
        let expected = usize::from(FINGERPRINTED.contains(path));
        assert_eq!(committed.matches(PLACEHOLDER).count(), expected, "{path}");
    }
    for (list, value) in [(modules(), served), (exported_modules(), fingerprint())] {
        for (path, text) in list {
            assert_eq!(text.matches(PLACEHOLDER).count(), 0, "{path}");
            let expected = usize::from(FINGERPRINTED.contains(path));
            assert_eq!(
                text.matches(&format!("\"{value}\"")).count(),
                expected,
                "{path}"
            );
            if !FINGERPRINTED.contains(path) {
                assert!(!text.contains(fingerprint()), "{path}");
            }
        }
    }
}

#[test]
fn admission_checks_and_strips_the_language_fingerprint() {
    let served = SERVED_FINGERPRINT.as_str();
    for language in [json!(served), json!(null)] {
        let mut value = json!({"plays": [], FINGERPRINT_PROPERTY: served, LANGUAGE_FINGERPRINT_PROPERTY: language});
        assert!(admit(&mut value).is_ok(), "{language}");
        assert_eq!(value, json!({"plays": []}));
    }
    for language in [
        json!(fingerprint()),
        json!(format!("{served} with lowering {}", fingerprint())),
    ] {
        let mut value = json!({"plays": [], FINGERPRINT_PROPERTY: served, LANGUAGE_FINGERPRINT_PROPERTY: language});
        assert!(admit(&mut value).is_err(), "{language}");
    }
}

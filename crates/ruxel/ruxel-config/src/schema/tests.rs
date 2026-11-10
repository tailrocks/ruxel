use super::{
    ENGINE, FINGERPRINT_PROPERTY, PLACEHOLDER, SERVED_FINGERPRINT, TREE, admit, exported_modules,
    exported_source, fingerprint, modules, source,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;

fn committed(dir: &Path, schema: &Path, found: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            committed(&path, schema, found);
        } else if path.extension().is_some_and(|extension| extension == "pkl")
            && let Ok(relative) = path.strip_prefix(schema)
        {
            found.insert(format!(
                "/{}",
                relative.to_string_lossy().replace('\\', "/")
            ));
        }
    }
}

#[test]
fn every_committed_module_is_served_and_exported() {
    let schema = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../schema"));
    let mut on_disk = BTreeSet::new();
    committed(&schema.join("ruxel"), schema, &mut on_disk);
    let tree: BTreeSet<String> = TREE.iter().map(|(path, _)| (*path).to_owned()).collect();
    assert_eq!(on_disk, tree);
    for list in [modules(), exported_modules()] {
        let listed: BTreeSet<String> = list.iter().map(|(path, _)| (*path).to_owned()).collect();
        assert_eq!(listed, tree);
    }
}

#[test]
fn served_and_exported_engine_replace_the_single_placeholder() {
    let committed = TREE
        .iter()
        .find(|(path, _)| *path == ENGINE)
        .map_or("", |(_, text)| *text);
    assert_eq!(
        committed,
        include_str!("../../../../../schema/ruxel/engine/Engine.pkl")
    );
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
    for (path, text) in modules().iter().chain(exported_modules()) {
        if *path != ENGINE {
            assert!(!text.contains(fingerprint()), "{path}");
        }
    }
}

#[test]
fn every_served_module_parses_without_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    for (path, text) in modules() {
        let tokens = pklr::lexer::lex(text)?;
        pklr::parser::parse_named(&tokens, text, path)?;
    }
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
        assert!(
            error.contains("modulepath:/ruxel/engine/Engine.pkl"),
            "{error}"
        );
        assert!(error.contains("modulepath:/ruxel/Workflow.pkl"), "{error}");
        assert!(error.contains("ruxel schema export"), "{error}");
    }
}

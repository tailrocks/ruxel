use super::{eval, eval_source};
use std::path::Path;

#[test]
fn parsed_module_inventory_covers_import_forms() -> Result<(), Box<dyn std::error::Error>> {
    for (source, expected) in [
        (r#"amends "base.pkl""#, vec![("base.pkl", false)]),
        (r#"extends "base.pkl""#, vec![("base.pkl", false)]),
        (r#"import "one.pkl" as one"#, vec![("one.pkl", false)]),
        (r#"import* "*.pkl" as all"#, vec![("*.pkl", true)]),
        (r#"value = import("one.pkl")"#, vec![("one.pkl", false)]),
        (r#"value = import*("*.pkl")"#, vec![("*.pkl", true)]),
        (
            r#"value = "\(import("one.pkl")) \(import*("*.pkl"))""#,
            vec![("one.pkl", false), ("*.pkl", true)],
        ),
        (
            r#"function f() = import*("nested/*.pkl")"#,
            vec![("nested/*.pkl", true)],
        ),
        (r#"value = read("file:resource.txt")"#, vec![]),
        (r##"value = import(#"raw.pkl"#)"##, vec![("raw.pkl", false)]),
        (r#"value = "import(\"fake.pkl\")""#, vec![]),
    ] {
        let tokens = pklr::lexer::lex(source)?;
        let module = pklr::parser::parse_named(&tokens, source, "fixture.pkl")?;
        assert_eq!(
            module.module_references().collect::<Vec<_>>(),
            expected,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn minimal_evaluates() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/minimal.pkl"
        )))
        .is_ok()
    );
}

#[test]
fn source_evaluation_preserves_output_and_relative_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("ruxel-source-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("main.pkl");
    let source = "import \"dependency.pkl\" as dep\noutput { value = dep.value }";
    std::fs::write(dir.join("dependency.pkl"), "value = \"rendered\"")?;
    std::fs::write(&path, source)?;
    assert_eq!(eval(&path)?, serde_json::json!("rendered"));
    assert_eq!(eval_source(source, &path)?, eval(&path)?);
    std::fs::write(&path, "output { value = \"replacement\" }")?;
    assert_eq!(eval_source(source, &path)?, serde_json::json!("rendered"));
    assert_eq!(eval(&path)?, serde_json::json!("replacement"));
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn environment_reads_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/env.pkl"
        )))
        .is_err()
    );
}

#[test]
fn network_imports_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/network.pkl"
        )))
        .is_err()
    );
}

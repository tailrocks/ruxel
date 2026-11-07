use super::{agent, controller, schema};
use anyhow::Result;
use std::collections::BTreeSet;
fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
#[test]
fn operation_classes_and_duplicates() -> Result<()> {
    let source = "typealias Operation = Copy|File\nclass Value { fixed kind = \"value\" }\nclass Copy { fixed kind = \"copy\"; path: String }\nclass File { fixed kind = \"file\" }";
    assert_eq!(schema::kinds(source)?, set(&["copy", "file"]));
    assert!(
        schema::kinds(&source.replace("fixed kind = \"file\"", "fixed kind = \"copy\"")).is_err()
    );
    assert!(schema::kinds(&source.replace("Copy|File", "Copy|Missing")).is_err());
    Ok(())
}
#[test]
fn controller_dispatch_and_negative_cases() -> Result<()> {
    let operation = "if matches!(kind, \"assert\" | \"debug\") { return self.local_retry(";
    let local = "async fn local_once() { match kind {\n \"assert\" => {},\n \"debug\" => {},\n _ => {}\n } } async fn local_retry() { self.local_once(); for attempt in 1..=attempts { self.local_once(); let scope = result_scope(";
    assert_eq!(
        controller::kinds(operation, local)?,
        set(&["assert", "debug"])
    );
    let cases = [
        (operation.replace(" | \"debug\"", ""), local.to_owned()),
        (operation.replace("local_retry", "other"), local.to_owned()),
        (format!("{operation}{operation}"), local.to_owned()),
        (
            operation.to_owned(),
            local.replace("\"debug\" =>", "\"other\" =>"),
        ),
        (
            operation.to_owned(),
            local.replace("self.local_once(", "self.other("),
        ),
        (
            operation.to_owned(),
            local.replace(
                "for attempt in 1..=attempts { self.local_once(",
                "for attempt in 1..=attempts { self.other(",
            ),
        ),
        (format!("/* {operation} */"), local.to_owned()),
        (
            format!("let inert = r#\"\n{operation}\n\"#;"),
            local.to_owned(),
        ),
        (
            format!("let inert = \"{}\";", operation.replace('"', "\\\"")),
            local.to_owned(),
        ),
        (
            format!("/* outer /* nested */ {operation} */"),
            local.to_owned(),
        ),
        (
            operation.to_owned(),
            local.replace(
                "for attempt in 1..=attempts { self.local_once(",
                "for attempt in 1..=attempts { /* self.local_once( */ self.other(",
            ),
        ),
        (
            operation.to_owned(),
            local.replace("async fn local_once", "async fn other"),
        ),
    ];
    for (op, body) in cases {
        assert!(
            controller::kinds(&op, &body).is_err(),
            "unexpectedly accepted {op}"
        );
    }
    Ok(())
}
#[test]
fn admission_guard() -> Result<()> {
    let source =
        "fn is_implemented(module: &str) -> bool { matches!(module, \"copy\" | \"file\") }";
    assert_eq!(agent::admission(source)?, Some(set(&["copy", "file"])));
    assert_eq!(agent::admission("fn execute() {}")?, None);
    assert!(agent::admission("fn is_implemented(x: &str) -> bool { custom(x) }").is_err());
    Ok(())
}
#[test]
fn actual_dispatch_not_unrelated_list() -> Result<()> {
    let source = "let result = match module {\n \"copy\" | \"template\" => copy::run(params),\n \"file\" => file::run(params),\n _ => unreachable!(),\n};\nfn is_implemented() { \"missing\"; }";
    assert_eq!(agent::dispatch(source)?, set(&["copy", "template", "file"]));
    assert!(agent::dispatch("fn is_implemented() { \"copy\"; }").is_err());
    Ok(())
}
